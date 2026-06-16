//! GPUI-side ops for the "drag a Pane header onto a Pane half to re-split"
//! affordance. The pure tree transform lives in `pane_tree.rs`
//! (`rearrange_pane` / `compute_drop_half` / `collect_pane_rects`); this
//! file owns the drag payload, the floating ghost, and the three Workspace
//! methods the render closures dispatch to. Transient hover state is the
//! single source of truth in `MainAreaContext.pane_drop_hover`.

use gpui::{
    Context, DragMoveEvent, IntoElement, Pixels, Point, Render, SharedString, Window, div,
    prelude::*, px,
};

use crate::ui::theme;
use crate::workspace::Workspace;

use super::pane_tree::{PaneId, collect_pane_rects, compute_drop_half, rearrange_pane};

/// Drag payload carried by GPUI while a Pane header is dragged. Minimal
/// + Clone; transient hover state lives in `MainAreaContext.pane_drop_hover`.
#[derive(Clone)]
pub(in crate::workspace) struct PaneHeaderDrag {
    pub dragged: PaneId,
    pub title: SharedString,
}

/// Floating ghost rendered next to the cursor during the drag — a small pill
/// showing the dragged pane's title. Mirrors `DraggedPanelTabGhost` /
/// `PathDrag`. GPUI anchors the ghost's top-left at `cursor - offset` (the
/// grab point within the header); padding the inner pill back by `offset`
/// re-pins it to the cursor so it doesn't drift away from the pointer.
pub(in crate::workspace) struct PaneHeaderDragGhost {
    pub title: SharedString,
    pub offset: Point<Pixels>,
}

impl Render for PaneHeaderDragGhost {
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
                    .text_color(theme::TEXT_PRIMARY)
                    .bg(t.panel_tab_drop_target_bg)
                    .rounded(px(theme::MODAL_BUTTON_RADIUS))
                    .child(self.title.clone()),
            )
    }
}

impl Workspace {
    /// Called from the pane-area `on_drag_move`. Compute the target pane and
    /// half from the cursor position local to the pane area, then store the
    /// hover (notifying only when it changes, per the cached-view rule).
    pub(in crate::workspace) fn update_pane_drag_from_move(
        &mut self,
        event: &DragMoveEvent<PaneHeaderDrag>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dragged = event.drag(cx).dragged;
        // Zoom mode renders a synthetic single-leaf layout, but the rects
        // below come from the full tree — hit-testing would target a hidden
        // pane. Disable drag-to-split while a pane is zoomed.
        if self.main_area.zoomed_pane_id.is_some() {
            if self.main_area.pane_drop_hover.take().is_some() {
                cx.notify();
            }
            return;
        }
        let local_x = f32::from(event.event.position.x - event.bounds.origin.x);
        let local_y = f32::from(event.event.position.y - event.bounds.origin.y);
        let w = f32::from(event.bounds.size.width);
        let h = f32::from(event.bounds.size.height);

        let Some(tab) = self.main_area.tabs.get(self.main_area.active_tab_index) else {
            return;
        };
        let mut rects = Vec::new();
        collect_pane_rects(&tab.layout, 0.0, 0.0, w, h, &mut rects);
        let hover = rects
            .iter()
            .find(|r| {
                local_x >= r.x && local_x < r.x + r.w && local_y >= r.y && local_y < r.y + r.h
            })
            // Don't highlight the dragged pane itself.
            .filter(|r| r.id != dragged)
            .and_then(|r| {
                compute_drop_half(local_x - r.x, local_y - r.y, r.w, r.h).map(|half| (r.id, half))
            });

        if self.main_area.pane_drop_hover != hover {
            self.main_area.pane_drop_hover = hover;
            cx.notify();
        }
    }

    /// Called from a pane's `on_drop`. Uses the stored hover (target + half)
    /// to move the dragged leaf next to the target via the pure transform.
    pub(in crate::workspace) fn drop_pane_onto(
        &mut self,
        dragged: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Keyboard zoom can fire between the last on_drag_move and this drop.
        // Mirror the guard in update_pane_drag_from_move: never rearrange the
        // full tree while a synthetic single-leaf zoom layout is showing.
        if self.main_area.zoomed_pane_id.is_some() {
            if self.main_area.pane_drop_hover.take().is_some() {
                cx.notify();
            }
            return;
        }
        let Some((target, half)) = self.main_area.pane_drop_hover.take() else {
            cx.notify();
            return;
        };
        // The dragged == target no-op is handled inside `rearrange_pane`.
        let moved = self.mutate_durable(cx, |ws, _| {
            ws.main_area
                .tabs
                .get_mut(ws.main_area.active_tab_index)
                .map(|t| rearrange_pane(&mut t.layout, dragged, target, half))
                .unwrap_or(false)
        });
        if moved {
            self.set_focused_pane(dragged, cx);
            self.focus_pane(dragged, window, cx);
            self.resize_all_tabs(window, cx);
        }
        // Notify even on a no-op move: take() above cleared pane_drop_hover,
        // so the overlay must be removed from screen regardless.
        cx.notify();
    }

    /// Called from the root `on_mouse_move` `!ev.dragging()` branch to clear
    /// a stale overlay when a drag was released outside the window.
    pub(in crate::workspace) fn clear_pane_drop_hover(&mut self, cx: &mut Context<Self>) {
        if self.main_area.pane_drop_hover.take().is_some() {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::pane_tree::{DropHalf, compute_drop_half};

    // The Workspace methods above need GPUI test infra; the pure hover
    // mapping they rely on is covered in `pane_tree.rs`. Re-assert one
    // `compute_drop_half` quadrant here to anchor the dependency: this
    // module stores whatever half the geometry returns into
    // `pane_drop_hover`, so the geometry contract is load-bearing here.
    #[test]
    fn drop_half_west_quadrant_drives_hover_target() {
        let (w, h) = (100.0, 100.0);
        assert_eq!(compute_drop_half(5.0, 50.0, w, h), Some(DropHalf::West));
    }
}
