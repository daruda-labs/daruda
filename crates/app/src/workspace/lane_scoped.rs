//! Per-lane state removed together by lane and project teardown.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use crate::files::gitignore::GitignoreSet;
use crate::files::tree::FileTree;
use crate::files::watcher::FileTreeWatcher;
use crate::lane::git::{GitDirs, GitTracking, GitWorktreeStatus};
use crate::lane::history::HistoryBuffer;
use crate::workspace::left_dock::file_tree_ops::{FilesReloadQueue, VisibleEntry};

#[derive(Default)]
pub(in crate::workspace) struct LaneScoped {
    pub git: GitLaneState,
    pub files: FileTreeLaneState,
    pub input_history: HistoryBuffer,
}

#[derive(Default)]
pub(in crate::workspace) struct FileTreeLaneState {
    pub tree: Option<FileTree>,
    /// Declared before the reload queue so the kernel watch stops first.
    pub watcher: Option<FileTreeWatcher>,
    pub reload_queue: Option<FilesReloadQueue>,
    pub visible_cache: Option<Arc<Vec<VisibleEntry>>>,
    pub gitignore: Option<GitignoreSet>,
}

/// Whether a refresh of one axis is running, and whether another request
/// arrived while it was. One value rather than two booleans, so
/// "queued but nothing running" cannot be written down.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum RefreshSlot {
    #[default]
    Idle,
    Running {
        repeat: bool,
    },
}

impl RefreshSlot {
    /// Claim the slot. `false` means a run was already in flight and this
    /// request was folded into it instead.
    pub fn claim(&mut self) -> bool {
        match self {
            Self::Idle => {
                *self = Self::Running { repeat: false };
                true
            }
            Self::Running { repeat } => {
                *repeat = true;
                false
            }
        }
    }

    /// Release the slot, reporting whether a request arrived mid-run and
    /// still needs a pass of its own.
    pub fn release(&mut self) -> bool {
        let pending = matches!(self, Self::Running { repeat: true });
        *self = Self::Idle;
        pending
    }
}

/// Result of the one-shot `rev-parse` that locates a lane's git dirs.
/// A separate state rather than an `Option`, so a probe already running and
/// a probe that failed cannot be mistaken for one never started — the
/// difference decides whether the next poll spawns another subprocess.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum GitDirsState {
    #[default]
    Unknown,
    Probing,
    Known(GitDirs),
    /// The probe failed. Watching is skipped; every daruda-run op still
    /// refreshes through the git lock.
    Unavailable,
}

/// The two git read axes are cached and refreshed independently: a ref
/// move (fetch, push, branch switch) invalidates only `tracking`, a
/// working-tree edit only `worktree`.
#[derive(Default)]
pub(in crate::workspace) struct GitLaneState {
    /// Missing until a tracking read succeeds, even if other git state exists.
    pub tracking: Option<GitTracking>,
    /// Missing until a worktree status read succeeds.
    pub worktree: Option<GitWorktreeStatus>,
    pub tracking_refresh: RefreshSlot,
    pub worktree_refresh: RefreshSlot,
    /// Where this lane's git state lives on disk — needed to watch it.
    pub dirs: GitDirsState,
    /// Lane-relative directory groups, kept only for this app session.
    pub collapsed_dirs: HashSet<String>,
    pub cursor: Option<GitCursor>,
}

/// Where the Git Changes keyboard cursor sits.
pub(in crate::workspace) struct GitCursor {
    /// Repo-root-relative path of the file the cursor is on — its identity,
    /// not its position. Space and Enter resolve through this, so a background
    /// refresh that inserts a file above cannot make them act on a different
    /// file than the highlighted one.
    pub path: PathBuf,
    /// Position `path` held among the visible files the last time it was one
    /// of them. Read only once `path` has left the list — discarded, reverted
    /// by an agent, hidden under a collapsed group — so the next arrow key
    /// resumes at the row that took its place instead of jumping to the top.
    pub index: usize,
}

#[cfg(test)]
#[path = "tests/lane_scoped.rs"]
mod tests;
