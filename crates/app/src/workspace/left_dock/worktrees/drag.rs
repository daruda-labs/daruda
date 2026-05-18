//! Drag payload + ghost preview for worktree rows.
//!
//! `DraggedWorktree` is the value passed through GPUI's `on_drag` /
//! `on_drop` chain. `DraggedWorktreeGhost` is the minimal element
//! shown under the cursor while a row is being dragged.

use crate::ui::theme;
use gpui::{Context, IntoElement, Render, SharedString, Window, div, prelude::*, px};

use daruda_store::project::WorktreeId;

/// Data carried by a worktree row during a drag operation. The
/// `label` is used by the ghost preview so it does not need to
/// re-read the worktree list while the drag is in flight.
#[derive(Clone, Debug)]
pub(in crate::workspace) struct DraggedWorktree {
    pub id: WorktreeId,
    pub label: SharedString,
}

/// Minimal ghost element shown under the cursor while a row is being
/// dragged. Renders a single-line label styled to match the left dock
/// row.
pub(super) struct DraggedWorktreeGhost {
    pub(super) label: SharedString,
}

impl Render for DraggedWorktreeGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        div()
            .px(px(theme::WORKTREE_ROW_PAD_X))
            .py(px(theme::WORKTREE_DRAG_GHOST_PAD_Y))
            .text_size(px(theme::WORKTREE_LABEL_FONT_SIZE))
            .text_color(t.dock_view_tab_active)
            .bg(t.worktree_row_hover_bg)
            .rounded(px(theme::MODAL_BUTTON_RADIUS))
            .child(self.label.clone())
    }
}
