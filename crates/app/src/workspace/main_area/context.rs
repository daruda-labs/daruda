use std::collections::HashMap;

use daruda_store::project::LaneRef;

use super::pane::{Pane, TabEntry};
use super::pane_tree::PaneId;
use crate::workspace::LaneRuntime;
use crate::workspace::layout::ops::{ContextMenuAnchor, DividerDrag};

/// Runtime state of the active lane's main area — TabBar + PaneTree.
///
/// Grouped out of `Workspace` so the tab/pane fields have a clear
/// conceptual boundary and the migration path to a GPUI Entity is
/// straightforward (the struct becomes the Entity's state verbatim).
///
/// `inactive_worktree_runtimes` lives here because it is the frozen
/// mirror of this struct: activating a lane swaps those same fields
/// in and out of `MainAreaContext`.
#[derive(Default)]
pub(in crate::workspace) struct MainAreaContext {
    pub tabs: Vec<TabEntry>,
    pub panes: Vec<Pane>,
    pub active_tab_index: usize,
    pub tab_history: Vec<usize>,
    pub focused_pane_id: PaneId,
    pub pending_resize: bool,
    /// Monotonic counter incremented every time a pane gains focus.
    /// Used by directional navigation (nav.rs) as the tie-breaker.
    pub activity_counter: HashMap<PaneId, u64>,
    pub activity_tick: u64,
    pub last_viewport: Option<(f32, f32)>,
    /// Active pane-divider drag. `None` = no drag in progress.
    pub drag_state: Option<DividerDrag>,
    /// Active right-click context menu. `None` = closed.
    pub context_menu: Option<ContextMenuAnchor>,
    /// When `Some(id)`, that pane is rendered full-size; all others hidden.
    pub zoomed_pane_id: Option<PaneId>,
    /// Runtime tab/pane state of every inactive lane. The active
    /// lane's runtime lives in the fields above; `activate_worktree`
    /// swaps those with the entry in this map.
    pub inactive_worktree_runtimes: HashMap<LaneRef, LaneRuntime>,
}
