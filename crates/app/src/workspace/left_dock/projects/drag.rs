//! Drag payload + ghost preview for the lanes view tree.
//!
//! `DragPayload` discriminates the three drag sources the tree
//! supports — a lane row, a project header, and a group header.
//! Each carries its own identifier alongside a display label so the
//! ghost preview can render without re-reading the workspace mid-flight.
//!
//! Drop handlers branch on the variant to enforce the placement rules
//! from the multi-project plan (lanes stay in their project,
//! projects move freely between groups and the top-level pool, groups
//! reorder only at the top level).

use crate::ui::theme;
use gpui::{Context, IntoElement, Render, SharedString, Window, div, prelude::*, px};

use daruda_store::project::{GroupId, LaneRef, ProjectId};

/// Payload exchanged through GPUI's `on_drag` / `on_drop` chain. The
/// label is purely for the ghost preview; the identifier carries the
/// actual move semantics.
#[derive(Clone, Debug)]
pub(in crate::workspace) enum DragPayload {
    Lane {
        target: LaneRef,
        label: SharedString,
    },
    Project {
        id: ProjectId,
        label: SharedString,
    },
    Group {
        id: GroupId,
        label: SharedString,
    },
}

impl DragPayload {
    pub(in crate::workspace) fn label(&self) -> SharedString {
        match self {
            DragPayload::Lane { label, .. }
            | DragPayload::Project { label, .. }
            | DragPayload::Group { label, .. } => label.clone(),
        }
    }
}

/// Minimal ghost element shown under the cursor while a row is being
/// dragged. Renders a single-line label styled to match the left dock
/// row.
pub(super) struct DragGhost {
    pub(super) label: SharedString,
}

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        div()
            .px(px(theme::LANE_ROW_PAD_X))
            .py(px(theme::LANE_DRAG_GHOST_PAD_Y))
            .text_size(px(theme::LANE_LABEL_FONT_SIZE))
            .text_color(t.dock_view_tab_active)
            .bg(t.lane_row_hover_bg)
            .rounded(px(theme::MODAL_BUTTON_RADIUS))
            .child(self.label.clone())
    }
}
