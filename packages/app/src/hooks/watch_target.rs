//! A path a watcher filters its events by, held in both spellings events
//! arrive in.
//!
//! FSEvents reports a resolved path (`/private/var/…` for `/var/…`, a
//! symlinked `~/.claude` as its target), while inotify and
//! ReadDirectoryChangesW report the watched path as it was given, joined with
//! the entry's name. A filter holding one spelling drops every event on the
//! platforms that use the other.

use std::path::{Path, PathBuf};

use daruda_core::path::canonicalize_or_self;

pub(crate) struct WatchTarget {
    given: PathBuf,
    resolved: PathBuf,
}

impl WatchTarget {
    /// `target`, resolved up to `anchor` — the nearest ancestor that exists,
    /// which is all that can be resolved while the target itself is missing.
    pub(crate) fn new(target: &Path, anchor: Option<&Path>) -> Self {
        Self {
            given: target.to_path_buf(),
            resolved: resolved_for(target, anchor),
        }
    }

    /// Whether `path` is the target or lies under it, in either spelling.
    pub(crate) fn contains(&self, path: &Path) -> bool {
        path.starts_with(&self.resolved) || path.starts_with(&self.given)
    }

    /// Whether `path` is the target itself, in either spelling.
    pub(crate) fn is(&self, path: &Path) -> bool {
        path == self.resolved || path == self.given
    }
}

/// Canonical up to the anchor, then the target's own components below it —
/// enough for a `starts_with` against every future resolved event.
fn resolved_for(target: &Path, anchor: Option<&Path>) -> PathBuf {
    let Some(anchor) = anchor else {
        return canonicalize_or_self(target);
    };
    let canonical_anchor = canonicalize_or_self(anchor);
    match target.strip_prefix(anchor) {
        Ok(tail) if !tail.as_os_str().is_empty() => canonical_anchor.join(tail),
        _ => canonical_anchor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A target that does not exist yet is watched through its nearest
    /// ancestor, and inotify / ReadDirectoryChangesW report events in the
    /// spelling it was given — those have to match on every platform.
    #[test]
    fn a_missing_target_matches_events_in_its_given_spelling() {
        let temp = tempfile::tempdir().unwrap();
        let anchor = temp.path().join(".claude");
        std::fs::create_dir(&anchor).unwrap();
        let skills = anchor.join("skills");

        let target = WatchTarget::new(&skills, Some(&anchor));

        assert!(target.contains(&skills.join("a").join("SKILL.md")));
        assert!(target.is(&skills));
        assert!(!target.contains(&anchor.join("other")));
    }

    /// A symlinked `~/.claude` (a dotfile manager's layout): FSEvents names
    /// the link's target, inotify the link — both are the target's events.
    #[cfg(unix)]
    #[test]
    fn an_event_matches_through_either_spelling() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("dotfiles");
        std::fs::create_dir(&real).unwrap();
        let link = temp.path().join(".claude");
        daruda_core::path::symlink(&real, &link).unwrap();
        let skills = link.join("skills");

        let target = WatchTarget::new(&skills, Some(&link));

        let resolved = daruda_core::path::canonicalize(&real).unwrap();
        assert!(target.contains(&resolved.join("skills/a/SKILL.md")));
        assert!(target.contains(&link.join("skills/a/SKILL.md")));
        assert!(!target.contains(&link.join("other")));
        assert!(target.is(&resolved.join("skills")));
        assert!(target.is(&skills));
    }
}
