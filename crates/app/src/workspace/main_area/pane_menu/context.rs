//! The snapshot a menu is derived from. Plain data — no entities, no
//! `Window` — so [`super::sections::compose`] is a pure function over it and
//! `render()` never re-enters an entity to build a menu.

use daruda_terminal::session::interval_tree::{LineRange, MarkId};
use daruda_terminal::view::TerminalLink;
use gpui::SharedString;

use crate::workspace::main_area::pane_tree::PaneId;

/// Upper bound on a selection routed to another pane. Mirrors iTerm2's
/// `kMaxSelectedTextLengthForCustomActions` — past this the composer stalls
/// and the user almost certainly did not mean to send it.
pub(super) const SEND_SELECTION_LIMIT: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PaneRole {
    /// The only leaf in its tab, so closing it closes the tab and zoom is a
    /// no-op. Drives both the missing Zoom entry and the close label.
    Solo,
    InSplit {
        zoomed: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LaneAccess {
    Accessible,
    Inaccessible,
}

/// What sits under the click. **Not exclusive** — an annotation covers a line
/// range while a link covers cells, so both can be present at once. Two
/// independent `Option`s rather than one enum for exactly that reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClickInfo {
    pub(super) link: Option<TerminalLink>,
    pub(super) annotation: Option<MarkId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SendTarget {
    pub(super) pane_id: PaneId,
    pub(super) label: SharedString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PaneMenuKind {
    Terminal {
        annotation_range: Option<LineRange>,
    },
    AgentChat {
        busy: bool,
    },
    /// `selected` is whether the graph has exactly one node selected — deleting
    /// needs one, and the menu opens with or without. `dep_selected` is the
    /// same question for a line: clicking one selects it and draws it in the
    /// accent, so what would be removed is visible before the row is chosen.
    FlowGraph {
        selected: bool,
        dep_selected: bool,
    },
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PaneMenuContext {
    pub(super) pane_id: PaneId,
    pub(super) role: PaneRole,
    pub(super) lane: LaneAccess,
    /// Captured *before* focus moves to the menu target, because clicking a
    /// menu item is a left-click outside the text block and clears the live
    /// selection first.
    pub(super) selection: Option<SharedString>,
    /// `None` for a pane-header right-click — there is no cell under it, so
    /// click-derived entries drop out without a separate code path.
    pub(super) click: Option<ClickInfo>,
    pub(super) send_targets: Vec<SendTarget>,
    pub(super) kind: PaneMenuKind,
}
