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
