//! GPUI-side ops for the "drag a tab cell along the tab bar to reorder it"
//! affordance, with a hover-to-switch preview: holding the drag over
//! another tab for `TAB_HOVER_SWITCH_DELAY` previews that tab (via
//! `Workspace::switch_tab_for_drag_preview`) without stealing focus or
//! persisting, so the user can see a tab's content before deciding where
//! to drop. Mirrors `pane_drag_ops.rs`'s split of responsibility — payload,
//! floating ghost, and the Workspace methods the render closures dispatch
//! to live here; the actual splice is `Workspace::move_tab` in
//! `tab_ops.rs`. Transient drag state is the single source of truth in
//! `MainAreaContext.{tab_hover_switch,tab_reorder_preview}`.
//!
//! Per-tab-cell listeners (not one container listener, unlike
//! `PaneHeaderDrag`): panes share `collect_pane_rects` across render,
//! divider-drag math, and drag hit-testing, so their `on_drag_move` does
//! manual hit-testing on the pane-area container. Tabs have no such
//! shared-geometry need — each cell's own `on_drag_move` already receives
//! its real GPUI-computed bounds via `DragMoveEvent.bounds`, so the
//! listener attaches directly to the cell instead of re-deriving the tab
//! bar's flex layout in pure Rust.

use std::time::Duration;

use gpui::{
    Context, DragMoveEvent, IntoElement, Pixels, Point, Render, SharedString, Window, div,
    prelude::*, px,
};

use crate::ui::theme;
use crate::workspace::Workspace;

/// Hover delay before the tab under the cursor becomes the active preview
/// during a drag. Matches GPUI's own tooltip hover-delay mechanism
/// (`window.spawn` + a background-executor timer): it fires deterministically
/// even if the mouse goes fully still mid-drag, unlike an elapsed-time check
/// only re-evaluated on `on_drag_move` (which never fires without motion).
const TAB_HOVER_SWITCH_DELAY: Duration = Duration::from_millis(600);

/// Drag payload carried while a tab cell is being dragged for reorder.
/// Minimal + Clone; transient drag state lives in `MainAreaContext`.
#[derive(Clone)]
pub(in crate::workspace) struct TabDrag {
    pub tab_id: u64,
    pub title: SharedString,
}

/// Floating ghost rendered next to the cursor during a tab drag — mirrors
/// `PaneHeaderDragGhost`'s pill styling exactly (same theme constants), just
/// a different payload type.
pub(in crate::workspace) struct TabDragGhost {
    pub title: SharedString,
    pub offset: Point<Pixels>,
}

impl Render for TabDragGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        div()
            .pl(self.offset.x + px(theme::DRAG_PILL_CURSOR_OFFSET))
            .pt(self.offset.y + px(theme::DRAG_PILL_CURSOR_OFFSET))
            .child(
                div()
                    .px(px(theme::DOCK_VIEW_TAB_PAD_X))
                    .py(px(theme::LANE_DRAG_GHOST_PAD_Y))
                    .text_size(px(theme::DOCK_VIEW_TAB_FONT_SIZE))
                    .text_color(t.text_primary)
                    .bg(t.panel_tab_drop_target_bg)
                    .rounded(px(theme::MODAL_BUTTON_RADIUS))
                    .child(self.title.clone()),
            )
    }
}

/// Pure midpoint math behind `update_tab_drag_from_move`: given the
/// cursor's x offset local to a tab cell and that cell's width, which
/// reorder-insertion slot the cursor implies (west half → insert before
/// this cell, east half → insert after). Returns `None` when `local_x`
/// falls outside `[0, cell_width]` — a stale/edge bounds report the caller
/// should ignore rather than snap to a wrong index.
fn reorder_target_index(local_x: f32, cell_width: f32, cell_index: usize) -> Option<usize> {
    if !(0.0..=cell_width).contains(&local_x) {
        return None;
    }
    Some(if local_x < cell_width / 2.0 {
        cell_index
    } else {
        cell_index + 1
    })
}

impl Workspace {
    /// Called from each tab cell's own `on_drag_move` — the cell reports
    /// its real GPUI-computed bounds directly (`event.bounds`), so no pure
    /// re-layout is needed (see module doc for why this differs from
    /// `PaneHeaderDrag`'s single-container listener).
    pub(in crate::workspace) fn update_tab_drag_from_move(
        &mut self,
        cell_tab_id: u64,
        cell_index: usize,
        event: &DragMoveEvent<TabDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let local_x = f32::from(event.event.position.x - event.bounds.origin.x);
        let w = f32::from(event.bounds.size.width);
        let Some(target_index) = reorder_target_index(local_x, w, cell_index) else {
            return;
        };
        if self.main_area.tab_reorder_preview != Some(target_index) {
            self.main_area.tab_reorder_preview = Some(target_index);
            cx.notify();
        }
        self.arm_tab_hover_switch(cell_tab_id, window, cx);
    }

    /// Arm (or re-arm) the hover-to-switch timer for `tab_id`. Hovering the
    /// same tab while a countdown is already running is a no-op — only a
    /// *different* `tab_id` restarts the delay. Overwriting
    /// `tab_hover_switch` drops the previous `Task`, which cancels it
    /// (mirrors GPUI's own tooltip-delay mechanism). Exposed at
    /// `pub(in crate::workspace)` (rather than private) so the
    /// stale-timer-cancellation guarantee is directly testable — the
    /// `DragMoveEvent` this is normally reached through cannot be
    /// constructed outside `gpui` itself (private fields), so tests must
    /// call this entry point directly.
    pub(in crate::workspace) fn arm_tab_hover_switch(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.main_area.tab_hover_switch.as_ref().map(|(id, _)| *id) == Some(tab_id) {
            return; // already counting down for this tab
        }
        let weak = cx.weak_entity();
        let task = window.spawn(cx, async move |cx| {
            cx.background_executor().timer(TAB_HOVER_SWITCH_DELAY).await;
            weak.update(cx, |ws, cx| {
                let still_armed =
                    ws.main_area.tab_hover_switch.as_ref().map(|(id, _)| *id) == Some(tab_id);
                if still_armed
                    && let Some(idx) = ws.active_runtime().tabs.iter().position(|t| t.id == tab_id)
                {
                    ws.switch_tab_for_drag_preview(idx, cx);
                }
            })
            .ok();
        });
        self.main_area.tab_hover_switch = Some((tab_id, task)); // drops (cancels) the old one
    }

    /// Called from a tab cell's `on_drop`. Consumes the live reorder
    /// preview index and commits the splice through `move_tab`, wrapped in
    /// `mutate_durable_in` so the reorder (unlike the preview switch) is
    /// persisted.
    pub(in crate::workspace) fn drop_tab_onto_bar(
        &mut self,
        dragged_tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.main_area.tab_hover_switch = None;
        let Some(target_index) = self.main_area.tab_reorder_preview.take() else {
            cx.notify();
            return;
        };
        let Some(from) = self
            .active_runtime()
            .tabs
            .iter()
            .position(|t| t.id == dragged_tab_id)
        else {
            cx.notify();
            return;
        };
        let to = if target_index > from {
            target_index - 1
        } else {
            target_index
        };
        self.mutate_durable_in(window, cx, |ws, _window, cx| ws.move_tab(from, to, cx));
        cx.notify();
    }

    /// Called from the root `on_mouse_move` `!ev.dragging()` branch
    /// (mirrors `clear_pane_drop_hover`) to clear a stale hover-switch
    /// timer and reorder-preview indicator when a drag was released
    /// outside the window.
    pub(in crate::workspace) fn clear_tab_drag_state(&mut self, cx: &mut Context<Self>) {
        // Both `.take()` calls must run unconditionally — `||` short-circuits
        // and would skip clearing `tab_reorder_preview` whenever
        // `tab_hover_switch` was already armed (the common case: hovering a
        // cell sets both), leaving a stale insertion-line indicator on
        // screen after the drag ends outside the window.
        let had_hover_switch = self.main_area.tab_hover_switch.take().is_some();
        let had_reorder_preview = self.main_area.tab_reorder_preview.take().is_some();
        if had_hover_switch || had_reorder_preview {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::reorder_target_index;

    #[test]
    fn reorder_target_index_west_half_targets_this_cell() {
        assert_eq!(reorder_target_index(5.0, 100.0, 2), Some(2));
    }

    #[test]
    fn reorder_target_index_east_half_targets_next_cell() {
        assert_eq!(reorder_target_index(60.0, 100.0, 2), Some(3));
    }

    #[test]
    fn reorder_target_index_exact_midpoint_rounds_to_east() {
        assert_eq!(reorder_target_index(50.0, 100.0, 0), Some(1));
    }

    #[test]
    fn reorder_target_index_outside_cell_bounds_is_none() {
        assert_eq!(reorder_target_index(-1.0, 100.0, 2), None);
        assert_eq!(reorder_target_index(101.0, 100.0, 2), None);
    }

    #[test]
    fn reorder_target_index_boundary_edges_are_inclusive() {
        assert_eq!(reorder_target_index(0.0, 100.0, 2), Some(2));
        assert_eq!(reorder_target_index(100.0, 100.0, 2), Some(3));
    }
}
