//! Left-dock Files view state owned by [`super::Workspace`].
//!
//! Groups the 12 fields that together drive the per-worktree lazy file
//! tree, its `notify` watcher, gitignore filtering, scrollbar handle,
//! and keyboard cursor. Workspace owns this struct directly (not as an
//! `Entity`) so existing access patterns compile without subscription
//! plumbing changes — the goal is **field grouping**, not actor
//! isolation. A future refactor can promote `FileTreeContext` to a
//! GPUI Entity once the call graph is mapped.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{FocusHandle, Task, UniformListScrollHandle};

use daruda_store::project::WorktreeRef;

use super::file_tree_ops::{FilesReloadQueue, VisibleEntry};
use crate::files::gitignore::GitignoreSet;
use crate::files::tree::{EntryId, FileTree};
use crate::files::watcher::FileTreeWatcher;

pub(in crate::workspace) struct FileTreeContext {
    /// Per-worktree lazy file tree for the left-dock Files view. Created
    /// on demand via `ensure_file_tree`. Independent of the live
    /// terminal/PTY state — purely a snapshot of the worktree's
    /// directory layout, refreshed on expand and (W-7g) by the
    /// `notify` watcher.
    ///
    /// Keyed by `WorktreeRef { project, worktree }` so two projects can
    /// each hold a worktree with id `0` without colliding.
    pub(in crate::workspace) file_trees: HashMap<WorktreeRef, FileTree>,

    /// Cached flattened visible-row list per worktree, fed straight to
    /// `uniform_list`. Rebuilt only at the seven trigger points
    /// enumerated in `file_tree_ops`. Other `cx.notify()` calls reuse
    /// the existing `Arc`.
    pub(in crate::workspace) files_visible_cache: HashMap<WorktreeRef, Arc<Vec<VisibleEntry>>>,

    /// Per-worktree `notify` watcher. Created on first
    /// `ensure_file_tree`; dropped when the worktree is removed (the
    /// `notify` watcher in turn stops the kernel watch).
    pub(in crate::workspace) file_watchers: HashMap<WorktreeRef, FileTreeWatcher>,

    /// Per-worktree FIFO of pending reloads. The drain task is
    /// serial — at most one in-flight `load_dir` per worktree.
    pub(in crate::workspace) files_reload_queues: HashMap<WorktreeRef, FilesReloadQueue>,

    /// Workspace-level polling task that fans out across every
    /// `file_watchers` entry. Held in a field so it stops when the
    /// `Workspace` entity drops.
    pub(in crate::workspace) files_watcher_poll: Option<Task<()>>,

    /// Mirror of `daruda_config::LeftDockConfig::files_show_hidden`.
    /// Drives the dotfile filter inside the Files view's `walk_into`.
    /// Toggled at runtime by `FilesToggleHidden`; `apply_config`
    /// overwrites it on live reload.
    pub(in crate::workspace) files_show_hidden: bool,

    /// Focus handle attached to the left-dock Files panel root. Click on
    /// any row routes focus here, which activates the `FilesPanel`
    /// key context — only then do `FilesSelectNext` etc. fire (so
    /// arrow keys do not steal from terminals).
    pub(in crate::workspace) files_panel_focus: FocusHandle,

    /// Keyboard cursor inside the Files view. Cleared on
    /// `activate_worktree`. Distinct from the focused file viewer's
    /// path — the cursor is a "highlighted but not opened" row.
    pub(in crate::workspace) files_selection: Option<EntryId>,

    /// Mirror of `daruda_config::LeftDockConfig::files_use_gitignore`.
    /// When true, `walk_into` consults `files_gitignore_index` per row
    /// and greys out ignored entries.
    pub(in crate::workspace) files_use_gitignore: bool,

    /// Mirror of `daruda_config::LeftDockConfig::file_icon_color_mode`.
    pub(in crate::workspace) files_icon_color_mode: daruda_config::IconColorMode,

    /// Compiled gitignore matcher per worktree. Built lazily in
    /// `ensure_file_tree`; rebuilt when `.gitignore` changes (watcher
    /// path filter inside `queue_files_event`).
    pub(in crate::workspace) files_gitignore_index: HashMap<WorktreeRef, GitignoreSet>,

    /// Scroll handle shared between the `uniform_list` and the
    /// dock's scrollbar overlay. Cloning is cheap (shared `Rc`).
    pub(in crate::workspace) files_scroll_handle: UniformListScrollHandle,
}
