//! Pure attribution of a scanned listening TCP port to the [`Lane`] whose
//! working directory the port's owning process is running under.
//!
//! Ported from Orca's `local-workspace-port-scanner.ts` /
//! `workspace-port-ownership.ts` matching rules — GPUI-free so it stays
//! unit-testable without a real OS port scan. Two-tier rule:
//!
//! 1. The port's owning process `cwd` is a lane path or a descendant of
//!    it — ties (nested worktrees) broken by picking the deepest path.
//! 2. Falls back to a word-boundary match of the lane path inside the
//!    process's command line, for processes that `chdir` away from the
//!    lane root but were launched from it.
//!
//! A port matching neither rule attributes to `None` rather than being
//! dropped, so callers can still surface "unattributed" ports.

use std::path::{Path, PathBuf};

/// A lane a port can be attributed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneCandidate {
    pub path: PathBuf,
    pub label: String,
}

/// A listening port discovered by the OS-level scan
/// (`workspace::sync::ports`), with the detail needed to attribute it to
/// a lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedPort {
    pub port: u16,
    pub cwd: Option<PathBuf>,
    pub command: Option<String>,
}

/// Which evidence attributed a port to a lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionConfidence {
    Cwd,
    Command,
}

/// Result of attributing one [`ScannedPort`] against a lane list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributedPort {
    pub port: u16,
    /// The matched lane's label, `None` when no lane claims this port.
    pub lane_label: Option<String>,
    pub confidence: Option<AttributionConfidence>,
}

/// Attribute every entry in `ports` to the deepest-matching lane in
/// `lanes`. See module docs for the matching rule.
pub fn attribute(ports: &[ScannedPort], lanes: &[LaneCandidate]) -> Vec<AttributedPort> {
    ports
        .iter()
        .map(|port| {
            let owner = attribute_one(port, lanes);
            AttributedPort {
                port: port.port,
                lane_label: owner.as_ref().map(|owner| owner.lane_label.clone()),
                confidence: owner.map(|owner| owner.confidence),
            }
        })
        .collect()
}

struct AttributedOwner {
    lane_label: String,
    confidence: AttributionConfidence,
}

fn attribute_one(port: &ScannedPort, lanes: &[LaneCandidate]) -> Option<AttributedOwner> {
    if let Some(cwd) = &port.cwd
        && let Some(lane) = deepest_matching(cwd, lanes)
    {
        return Some(AttributedOwner {
            lane_label: lane.label.clone(),
            confidence: AttributionConfidence::Cwd,
        });
    }

    let command = port.command.as_deref()?;
    deepest_matching_by(lanes, |lane| includes_path_boundary(command, &lane.path)).map(|lane| {
        AttributedOwner {
            lane_label: lane.label.clone(),
            confidence: AttributionConfidence::Command,
        }
    })
}

/// Among lanes whose path equals or contains `cwd` as an ancestor, pick
/// the one with the most path components — the deepest, i.e. most
/// specific, match. This is what makes a nested worktree win over its
/// parent when both contain `cwd`.
fn deepest_matching<'a>(cwd: &Path, lanes: &'a [LaneCandidate]) -> Option<&'a LaneCandidate> {
    deepest_matching_by(lanes, |lane| cwd.starts_with(&lane.path))
}

fn deepest_matching_by<F>(lanes: &[LaneCandidate], matches: F) -> Option<&LaneCandidate>
where
    F: Fn(&LaneCandidate) -> bool,
{
    lanes
        .iter()
        .filter(|lane| matches(lane))
        .max_by_key(|lane| lane.path.components().count())
}

/// True when `haystack` contains `needle`'s path string at a word
/// boundary — e.g. lane path `/repo/app` matches `cd /repo/app &&
/// npm start` but not `/repo/app-2`.
fn includes_path_boundary(haystack: &str, needle: &Path) -> bool {
    let needle = needle.to_string_lossy();
    if needle.is_empty() {
        return false;
    }
    // A byte continues the matched path's own name (rather than
    // terminating it) when it's alphanumeric or `_`/`-`/`.` — so
    // `/repo/app` does not match inside `/repo/app-2` or `/repo/app.old`.
    // `/` is a boundary: it starts a new path segment, so a file
    // *inside* the matched directory (`/repo/app/server.js`) still
    // counts as a match.
    let is_boundary = |byte: Option<u8>| match byte {
        None => true,
        Some(b) => !(b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')),
    };
    let mut search_start = 0;
    while let Some(rel_idx) = haystack[search_start..].find(needle.as_ref()) {
        let idx = search_start + rel_idx;
        let end = idx + needle.len();
        let before = if idx == 0 {
            None
        } else {
            Some(haystack.as_bytes()[idx - 1])
        };
        let after = haystack.as_bytes().get(end).copied();
        if is_boundary(before) && is_boundary(after) {
            return true;
        }
        search_start = idx + 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(path: &str, label: &str) -> LaneCandidate {
        LaneCandidate {
            path: PathBuf::from(path),
            label: label.to_string(),
        }
    }

    fn scanned(port: u16, cwd: Option<&str>, command: Option<&str>) -> ScannedPort {
        ScannedPort {
            port,
            cwd: cwd.map(PathBuf::from),
            command: command.map(str::to_string),
        }
    }

    #[test]
    fn matches_exact_cwd() {
        let lanes = vec![lane("/repo/app", "app")];
        let ports = vec![scanned(3000, Some("/repo/app"), None)];
        let result = attribute(&ports, &lanes);
        assert_eq!(result[0].lane_label.as_deref(), Some("app"));
        assert_eq!(result[0].confidence, Some(AttributionConfidence::Cwd));
    }

    #[test]
    fn matches_subdirectory_cwd() {
        let lanes = vec![lane("/repo/app", "app")];
        let ports = vec![scanned(3000, Some("/repo/app/src/server"), None)];
        let result = attribute(&ports, &lanes);
        assert_eq!(result[0].lane_label.as_deref(), Some("app"));
    }

    #[test]
    fn does_not_match_sibling_with_shared_prefix() {
        let lanes = vec![lane("/repo/app", "app")];
        let ports = vec![scanned(3000, Some("/repo/app-2"), None)];
        let result = attribute(&ports, &lanes);
        assert_eq!(result[0].lane_label, None);
    }

    #[test]
    fn nested_worktree_picks_deepest_match() {
        let lanes = vec![
            lane("/repo", "repo-main"),
            lane("/repo/nested-worktree", "repo-feature"),
        ];
        let ports = vec![scanned(3000, Some("/repo/nested-worktree/src"), None)];
        let result = attribute(&ports, &lanes);
        assert_eq!(result[0].lane_label.as_deref(), Some("repo-feature"));
        assert_eq!(result[0].confidence, Some(AttributionConfidence::Cwd));
    }

    #[test]
    fn falls_back_to_command_line_when_cwd_unmatched() {
        let lanes = vec![lane("/repo/app", "app")];
        let ports = vec![scanned(
            3000,
            Some("/tmp"),
            Some("node /repo/app/server.js"),
        )];
        let result = attribute(&ports, &lanes);
        assert_eq!(result[0].lane_label.as_deref(), Some("app"));
        assert_eq!(result[0].confidence, Some(AttributionConfidence::Command));
    }

    #[test]
    fn command_line_fallback_picks_deepest_match() {
        let lanes = vec![
            lane("/repo", "repo-main"),
            lane("/repo/worktrees/feature", "repo-feature"),
        ];
        let ports = vec![scanned(
            3000,
            Some("/tmp"),
            Some("node /repo/worktrees/feature/server.js"),
        )];
        let result = attribute(&ports, &lanes);
        assert_eq!(result[0].lane_label.as_deref(), Some("repo-feature"));
        assert_eq!(result[0].confidence, Some(AttributionConfidence::Command));
    }

    #[test]
    fn command_line_fallback_respects_word_boundary() {
        let lanes = vec![lane("/repo/app", "app")];
        let ports = vec![scanned(
            3000,
            Some("/tmp"),
            Some("node /repo/app-2/server.js"),
        )];
        let result = attribute(&ports, &lanes);
        assert_eq!(result[0].lane_label, None);
    }

    #[test]
    fn no_cwd_no_command_is_unattributed() {
        let lanes = vec![lane("/repo/app", "app")];
        let ports = vec![scanned(3000, None, None)];
        let result = attribute(&ports, &lanes);
        assert_eq!(result[0].lane_label, None);
        assert_eq!(result[0].confidence, None);
    }

    #[test]
    fn unmatched_port_returns_none_rather_than_dropped() {
        let lanes = vec![lane("/repo/app", "app")];
        let ports = vec![
            scanned(3000, Some("/repo/app"), None),
            scanned(4000, Some("/unrelated"), None),
        ];
        let result = attribute(&ports, &lanes);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].port, 4000);
        assert_eq!(result[1].lane_label, None);
    }
}
