use std::collections::HashMap;

use daruda_store::project::LaneRef;
use gpui::Task;

use super::pane_tree::{DropHalf, PaneId};
use crate::workspace::LaneRuntime;
use crate::workspace::layout::ops::{DividerDrag, PopupMenuDeploy};

/// Runtime state of the main area — TabBar + PaneTree — for every lane.
///
/// Grouped out of `Workspace` so the tab/pane state has a clear
/// conceptual boundary and the migration path to a GPUI Entity is
/// straightforward (the struct becomes the Entity's state verbatim).
///
/// `runtimes` is the single store: every lane's runtime lives here,
/// keyed by `LaneRef`. The active lane is just the entry under
/// `Workspace::active` — switching lanes re-points that key, never
/// moves data. `Workspace::active_runtime()` / `active_runtime_mut()`
/// resolve the active entry, which is guaranteed present (seeded at
/// construction and on every `activate_lane`).
#[derive(Default)]
pub(in crate::workspace) struct MainAreaContext {
    /// Every lane's live runtime (tabs / panes / focus / history),
    /// keyed by `LaneRef`. The active lane is the entry under
    /// `Workspace::active`; there is no separate active/inactive split.
    pub runtimes: HashMap<LaneRef, LaneRuntime>,
    pub pending_resize: bool,
    /// Monotonic counter incremented every time a pane gains focus.
    /// Used by directional navigation (nav.rs) as the tie-breaker.
    pub activity_counter: HashMap<PaneId, u64>,
    pub activity_tick: u64,
    pub last_viewport: Option<(f32, f32)>,
    /// Active pane-divider drag. `None` = no drag in progress.
    pub drag_state: Option<DividerDrag>,
    /// Active imperatively-opened PopupMenu (System B) — see `PopupMenuDeploy`.
    pub popup_menu_deploy: Option<PopupMenuDeploy>,
    /// When `Some(id)`, that pane is rendered full-size; all others hidden.
    pub zoomed_pane_id: Option<PaneId>,
    /// Transient hover target while a Pane header is being dragged:
    /// the pane under the cursor and which half it would split. Single
    /// source of truth for the drop-overlay; `None` = no active hover.
    pub pane_drop_hover: Option<(PaneId, DropHalf)>,
    /// Tab currently armed to switch-on-hover during a `TabDrag`, and the
    /// cancellable timer task backing it. Overwriting drops the previous
    /// `Task`, which cancels it (mirrors GPUI's own tooltip-delay mechanism).
    pub tab_hover_switch: Option<(u64, Task<()>)>,
    /// Live reorder-insertion index while a `TabDrag` is over the tab bar,
    /// tagged with the tab cell that set it so only that cell releases it
    /// again. `None` = not currently hovering a tab cell.
    pub tab_reorder_preview: Option<(u64, usize)>,
    /// Tab that was active before the in-flight drag's first hover preview,
    /// by id (indices shift under reorder/close). `finish_tab_drag` restores
    /// it unless the drag committed a drop. `None` = nothing to unwind.
    pub tab_preview_restore: Option<u64>,
}
