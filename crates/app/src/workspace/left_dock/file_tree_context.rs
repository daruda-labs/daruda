//! Left-dock Files view state owned by [`super::Workspace`].
//!
//! Groups the fields driving the per-lane lazy file tree, its `notify`
//! watcher, gitignore filtering, scrollbar handle, and keyboard cursor.
//! Owned directly by Workspace (not as an `Entity`).

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{FocusHandle, Task, UniformListScrollHandle};

use daruda_store::project::LaneRef;

use super::file_tree_ops::{FilesReloadQueue, VisibleEntry};
use crate::files::gitignore::GitignoreSet;
use crate::files::tree::{EntryId, FileTree};
use crate::files::watcher::FileTreeWatcher;

pub(in crate::workspace) struct FileTreeContext {
    /// Per-lane lazy file tree, created on demand via `ensure_file_tree`.
    /// A snapshot of the lane's directory layout (not live PTY state),
    /// refreshed on expand and by the `notify` watcher. Keyed by
    /// `LaneRef { project, lane }` so lane id `0` in two projects can't
    /// collide.
    pub(in crate::workspace) file_trees: HashMap<LaneRef, FileTree>,

    /// Cached flattened visible-row list per lane, fed to `uniform_list`.
    /// Rebuilt only at the trigger points listed in `file_tree_ops`;
    /// other `cx.notify()` calls reuse the existing `Arc`.
    pub(in crate::workspace) files_visible_cache: HashMap<LaneRef, Arc<Vec<VisibleEntry>>>,

    /// Per-lane `notify` watcher. Created on first `ensure_file_tree`;
    /// dropped when the lane is removed (which stops the kernel watch).
    pub(in crate::workspace) file_watchers: HashMap<LaneRef, FileTreeWatcher>,

    /// Per-lane FIFO of pending reloads. The drain task is
    /// serial — at most one in-flight `load_dir` per lane.
    pub(in crate::workspace) files_reload_queues: HashMap<LaneRef, FilesReloadQueue>,

    /// Workspace-level polling task that fans out across every
    /// `file_watchers` entry. Held in a field so it stops on drop.
    pub(in crate::workspace) files_watcher_poll: Option<Task<()>>,

    /// Focus handle for the Files panel root. A row click routes focus
    /// here, activating the `FilesPanel` key context so `FilesSelectNext`
    /// etc. fire without stealing arrow keys from terminals.
    pub(in crate::workspace) files_panel_focus: FocusHandle,

    /// Keyboard cursor inside the Files view (a "highlighted but not
    /// opened" row, distinct from the focused file viewer's path).
    /// Cleared on `activate_lane`.
    pub(in crate::workspace) files_selection: Option<EntryId>,

    /// Compiled gitignore matcher per lane. Built lazily in
    /// `ensure_file_tree`; rebuilt when `.gitignore` changes.
    pub(in crate::workspace) files_gitignore_index: HashMap<LaneRef, GitignoreSet>,

    /// Scroll handle shared between the `uniform_list` and the dock's
    /// scrollbar overlay. Cloning is cheap (shared `Rc`).
    pub(in crate::workspace) files_scroll_handle: UniformListScrollHandle,
}
