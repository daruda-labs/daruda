//! Lane-aware path conversion helpers.
//!
//! Git status paths are repo-root-relative, file-tree paths are lane-root
//! relative, and drag/external paths are absolute. `LanePaths` captures both
//! anchors so call sites choose the intended conversion explicitly instead of
//! doing raw `.join()` arithmetic.

use std::path::{Path, PathBuf};

/// Path-conversion helper for one lane.
pub struct LanePaths<'a> {
    /// Absolute path of the lane directory (`wt.path`).
    pub wt_path: &'a Path,
    /// Git repo root, or `None` for non-git lanes.
    pub repo_root: Option<&'a Path>,
}

impl<'a> LanePaths<'a> {
    /// `git status --porcelain` path (repo-root-relative) → absolute.
    ///
    /// Uses `repo_root` as the base; falls back to `wt_path` for non-git
    /// lanes so callers do not need to special-case them.
    pub fn from_git_status(&self, p: &Path) -> PathBuf {
        self.repo_root.unwrap_or(self.wt_path).join(p)
    }

    /// `FileTree` path (lane-root-relative) → absolute.
    pub fn from_files_tree(&self, p: &Path) -> PathBuf {
        self.wt_path.join(p)
    }

    /// Absolute path → lane-relative (for UI display and `git restore`).
    ///
    /// Returns `None` when `abs` does not start with `wt_path` (should not
    /// happen in normal operation; callers can log and fall back).
    pub fn to_wt_relative(&self, abs: &Path) -> Option<PathBuf> {
        abs.strip_prefix(self.wt_path).ok().map(Path::to_path_buf)
    }

    /// Absolute path → repo-root-relative (for `git add`, `git show :path`).
    ///
    /// Returns `None` for non-git worktrees or when `abs` does not start with
    /// `repo_root`.
    pub fn to_repo_relative(&self, abs: &Path) -> Option<PathBuf> {
        self.repo_root
            .and_then(|r| abs.strip_prefix(r).ok().map(Path::to_path_buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_git_status_uses_repo_root() {
        let wt = Path::new("/term/daruda");
        let repo = Path::new("/term");
        let wp = LanePaths {
            wt_path: wt,
            repo_root: Some(repo),
        };
        assert_eq!(
            wp.from_git_status(Path::new("daruda/crates/foo.rs")),
            PathBuf::from("/term/daruda/crates/foo.rs")
        );
    }

    #[test]
    fn from_git_status_falls_back_to_wt_when_no_repo() {
        let wt = Path::new("/projects/myapp");
        let wp = LanePaths {
            wt_path: wt,
            repo_root: None,
        };
        assert_eq!(
            wp.from_git_status(Path::new("src/main.rs")),
            PathBuf::from("/projects/myapp/src/main.rs")
        );
    }

    #[test]
    fn from_files_tree_always_uses_wt_path() {
        let wt = Path::new("/term/daruda");
        let repo = Path::new("/term");
        let wp = LanePaths {
            wt_path: wt,
            repo_root: Some(repo),
        };
        assert_eq!(
            wp.from_files_tree(Path::new("crates/foo.rs")),
            PathBuf::from("/term/daruda/crates/foo.rs")
        );
    }

    #[test]
    fn to_wt_relative_strips_prefix() {
        let wt = Path::new("/term/daruda");
        let wp = LanePaths {
            wt_path: wt,
            repo_root: None,
        };
        assert_eq!(
            wp.to_wt_relative(Path::new("/term/daruda/crates/foo.rs")),
            Some(PathBuf::from("crates/foo.rs"))
        );
    }

    #[test]
    fn to_wt_relative_returns_none_outside_wt() {
        let wt = Path::new("/term/daruda");
        let wp = LanePaths {
            wt_path: wt,
            repo_root: None,
        };
        assert!(wp.to_wt_relative(Path::new("/other/file.rs")).is_none());
    }

    #[test]
    fn to_repo_relative_strips_repo_prefix() {
        let wt = Path::new("/term/daruda");
        let repo = Path::new("/term");
        let wp = LanePaths {
            wt_path: wt,
            repo_root: Some(repo),
        };
        assert_eq!(
            wp.to_repo_relative(Path::new("/term/daruda/crates/foo.rs")),
            Some(PathBuf::from("daruda/crates/foo.rs"))
        );
    }

    #[test]
    fn to_repo_relative_none_for_non_git() {
        let wt = Path::new("/projects/myapp");
        let wp = LanePaths {
            wt_path: wt,
            repo_root: None,
        };
        assert!(
            wp.to_repo_relative(Path::new("/projects/myapp/src/main.rs"))
                .is_none()
        );
    }

    #[test]
    fn round_trip_same_path_when_wt_equals_repo() {
        let root = Path::new("/term");
        let wp = LanePaths {
            wt_path: root,
            repo_root: Some(root),
        };
        let git_rel = Path::new("crates/foo.rs");
        let abs = wp.from_git_status(git_rel);
        assert_eq!(abs, PathBuf::from("/term/crates/foo.rs"));
        assert_eq!(wp.to_wt_relative(&abs), Some(git_rel.to_path_buf()));
        assert_eq!(wp.to_repo_relative(&abs), Some(git_rel.to_path_buf()));
    }
}
