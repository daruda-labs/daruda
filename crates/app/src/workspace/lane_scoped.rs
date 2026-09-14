//! Per-lane state removed together by lane and project teardown.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use crate::files::gitignore::GitignoreSet;
use crate::files::tree::FileTree;
use crate::files::watcher::FileTreeWatcher;
use crate::lane::git::GitStatusData;
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

#[derive(Default)]
pub(in crate::workspace) struct GitLaneState {
    /// Missing until a status fetch succeeds, even if other git state exists.
    pub status: Option<GitStatusData>,
    /// At most one status fetch runs per lane; requests during it coalesce.
    pub fetch_in_flight: bool,
    /// Drained on completion to fetch changes that arrived during the run.
    pub fetch_pending_repeat: bool,
    /// Lane-relative directory groups, kept only for this app session.
    pub collapsed_dirs: HashSet<String>,
    /// Repo-root-relative path so refreshes keep the cursor on the same file.
    pub cursor: Option<PathBuf>,
}

#[cfg(test)]
#[path = "tests/lane_scoped.rs"]
mod tests;
