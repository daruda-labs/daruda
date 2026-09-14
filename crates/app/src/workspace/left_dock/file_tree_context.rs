//! Left-dock Files view state owned by [`super::Workspace`].
//!
//! Shared polling, focus, selection, and scrolling for the Files panel.
//! Per-lane data and watchers live in `LaneScoped::files`.

use gpui::{FocusHandle, Task, UniformListScrollHandle};

use crate::files::tree::EntryId;

pub(in crate::workspace) struct FileTreeContext {
    /// Workspace-level polling task that fans out across every
    /// lane's file watcher. Held in a field so it stops on drop.
    pub(in crate::workspace) files_watcher_poll: Option<Task<()>>,

    /// Focus handle for the Files panel root. A row click routes focus
    /// here, activating the `FilesPanel` key context so `FilesSelectNext`
    /// etc. fire without stealing arrow keys from terminals.
    pub(in crate::workspace) files_panel_focus: FocusHandle,

    /// Keyboard cursor inside the Files view (a "highlighted but not
    /// opened" row, distinct from the focused file viewer's path).
    /// Cleared on `activate_lane`.
    pub(in crate::workspace) files_selection: Option<EntryId>,

    /// Scroll handle shared between the `uniform_list` and the dock's
    /// scrollbar overlay. Cloning is cheap (shared `Rc`).
    pub(in crate::workspace) files_scroll_handle: UniformListScrollHandle,
}
