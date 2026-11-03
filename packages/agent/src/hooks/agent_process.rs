//! Find the agent process a hook fired for, from the hook's own ancestry.
//!
//! An agent runs its hook commands as descendants, so the agent is the
//! nearest ancestor that proves it runs the session: for Claude Code the
//! process whose `sessions/<pid>.json` names the session, for codex the
//! nearest process named `codex`. The PID lets the app confirm the agent's
//! exit; without one the session still reports status, only not exit.

use std::ffi::OsStr;
use std::path::Path;

use super::route::Harness;
use crate::pty_link;

/// Ancestor hops to inspect. A hook runs one or two shells below its agent;
/// the cap only bounds a malformed or cyclic parent chain.
const MAX_ANCESTORS: usize = 16;

/// The agent process running `session_id` above this hook process.
pub fn find(harness: Harness, session_id: &str) -> Option<u32> {
    let ancestors = ancestors(MAX_ANCESTORS);
    match harness {
        Harness::Claude => {
            let dir = pty_link::sessions_dir_in(&crate::accounts::claude::system_claude_home()?);
            nearest(&ancestors, |pid, _| holds_session(&dir, pid, session_id))
        }
        Harness::Codex => nearest(&ancestors, |_, name| is_codex(name)),
    }
}

/// The first ancestor, nearest first, that `is_agent` accepts.
fn nearest(ancestors: &[(u32, String)], is_agent: impl Fn(u32, &str) -> bool) -> Option<u32> {
    ancestors
        .iter()
        .find(|(pid, name)| is_agent(*pid, name))
        .map(|(pid, _)| *pid)
}

fn holds_session(dir: &Path, pid: u32, session_id: &str) -> bool {
    pty_link::read_session_meta_in(dir, pid).is_some_and(|meta| meta.session_id == session_id)
}

/// `codex` itself, however it was installed: the native binary is named
/// `codex` (`codex.exe` on Windows) under npm, Homebrew and app bundles.
fn is_codex(name: &str) -> bool {
    Path::new(name)
        .file_stem()
        .and_then(OsStr::to_str)
        .is_some_and(|stem| stem.eq_ignore_ascii_case("codex"))
}

/// `(pid, process name)` of this process's ancestors, parent first.
fn ancestors(limit: usize) -> Vec<(u32, String)> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let Ok(mut system) = System::new() else {
        return Vec::new();
    };
    let mut lookup = |pid: u32| {
        let target = Pid::from_u32(pid);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[target]),
            true,
            ProcessRefreshKind::nothing(),
        );
        system.process(target).map(|process| {
            (
                process.parent(),
                process.name().to_string_lossy().into_owned(),
            )
        })
    };
    let mut out = Vec::new();
    let mut current = lookup(std::process::id()).and_then(|(parent, _)| parent);
    while let Some(pid) = current.map(|p| p.as_u32()) {
        if out.len() >= limit || pid == 0 || out.iter().any(|(seen, _)| *seen == pid) {
            break;
        }
        let Some((parent, name)) = lookup(pid) else {
            break;
        };
        out.push((pid, name));
        current = parent;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> Vec<(u32, String)> {
        vec![
            (30, "zsh".into()),
            (20, "codex".into()),
            (10, "zsh".into()),
            (5, "codex".into()),
        ]
    }

    #[test]
    fn the_nearest_matching_ancestor_wins() {
        assert_eq!(nearest(&chain(), |_, name| is_codex(name)), Some(20));
        assert_eq!(nearest(&chain(), |pid, _| pid == 10), Some(10));
        assert_eq!(nearest(&chain(), |_, _| false), None);
    }

    #[test]
    fn codex_is_recognised_by_its_binary_name() {
        assert!(is_codex("codex"));
        assert!(is_codex("codex.exe"));
        assert!(is_codex("/usr/local/bin/codex"));
        assert!(!is_codex("codex-acp"));
        assert!(!is_codex("node"));
    }

    #[test]
    fn a_session_file_naming_the_session_proves_the_process() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("42.json"),
            r#"{"pid":42,"sessionId":"s1","cwd":"/tmp"}"#,
        )
        .unwrap();
        assert!(holds_session(dir.path(), 42, "s1"));
        assert!(!holds_session(dir.path(), 42, "s2"));
        assert!(!holds_session(dir.path(), 43, "s1"));
    }

    /// The walk reaches real processes: this test's own parent exists.
    #[test]
    fn the_ancestry_starts_at_the_parent_process() {
        let ancestors = ancestors(MAX_ANCESTORS);
        assert!(!ancestors.is_empty());
        assert!(ancestors.iter().all(|(pid, _)| *pid != std::process::id()));
    }
}
