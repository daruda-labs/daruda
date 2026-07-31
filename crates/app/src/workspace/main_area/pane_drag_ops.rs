//! GPUI-side ops for the "drag a Pane header onto a Pane half to re-split"
//! affordance, plus the sibling "drag a whole Tab onto a Pane half to merge
//! it in" affordance (`TabDrag` payload, owned by `tab_drag_ops.rs`). The
//! pure tree transforms live in `pane_tree.rs` (`rearrange_pane` /
//! `compute_drop_half` / `collect_pane_rects`) and `tab_ops.rs`
//! (`merge_tab_into_pane`); this file owns the drag payload, the floating
//! ghost, and the Workspace methods the render closures dispatch to for
//! both affordances — they share one hit-test core
//! (`compute_pane_drop_hover`) and one hover slot. Transient hover state is
//! the single source of truth in `MainAreaContext.pane_drop_hover`.

use gpui::{
    Context, DragMoveEvent, IntoElement, Pixels, Point, Render, SharedString, Window, div,
    prelude::*, px,
};

use crate::ui::theme;
use crate::workspace::Workspace;

use super::pane::TabEntry;
use super::pane_tree::{DropHalf, PaneId, collect_pane_rects, compute_drop_half, rearrange_pane};
use super::tab_drag_ops::TabDrag;

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
                    .text_color(t.text_primary)
                    .bg(t.panel_tab_drop_target_bg)
                    .rounded(px(theme::MODAL_BUTTON_RADIUS))
                    .child(self.title.clone()),
            )
    }
}

/// Shared hit-test core behind both `update_pane_drag_from_move` and
/// `update_tab_merge_hover_from_move`: given cursor position local to the
/// pane-area container, find which pane rect the cursor is over (excluding
/// `exclude`, when dragging a pane header onto its own tab) and compute the
/// drop half within it.
fn compute_pane_drop_hover(
    tab: &TabEntry,
    local_x: f32,
    local_y: f32,
    w: f32,
    h: f32,
    exclude: Option<PaneId>,
) -> Option<(PaneId, DropHalf)> {
    let mut rects = Vec::new();
    collect_pane_rects(&tab.layout, 0.0, 0.0, w, h, &mut rects);
    rects
        .iter()
        .find(|r| local_x >= r.x && local_x < r.x + r.w && local_y >= r.y && local_y < r.y + r.h)
        .filter(|r| Some(r.id) != exclude)
        .and_then(|r| {
            compute_drop_half(local_x - r.x, local_y - r.y, r.w, r.h).map(|half| (r.id, half))
        })
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

        let Some(tab) = self
            .active_runtime()
            .tabs
            .get(self.active_runtime().active_tab_index)
        else {
            return;
        };
        // Don't highlight the dragged pane itself.
        let hover = compute_pane_drop_hover(tab, local_x, local_y, w, h, Some(dragged));

        if self.main_area.pane_drop_hover != hover {
            self.main_area.pane_drop_hover = hover;
            cx.notify();
        }
    }

    /// Called from the pane-area `on_drag_move` while dragging a *tab* (not
    /// a pane header) — computes the merge-drop target/half the same way
    /// `update_pane_drag_from_move` does, but the self-merge guard is by tab
    /// identity rather than by excluding one pane rect: the active tab's
    /// panes are never the dragged tab's own panes unless the dragged tab
    /// *is* the active tab, in which case there is nothing to merge into.
    pub(in crate::workspace) fn update_tab_merge_hover_from_move(
        &mut self,
        event: &DragMoveEvent<TabDrag>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dragged_tab_id = event.drag(cx).tab_id;
        // Mirrors update_pane_drag_from_move's zoom guard: the rects below
        // come from the full tree, but zoom renders a synthetic single-leaf
        // layout — hit-testing would target a hidden pane.
        if self.main_area.zoomed_pane_id.is_some() {
            if self.main_area.pane_drop_hover.take().is_some() {
                cx.notify();
            }
            return;
        }
        let active_index = self.active_runtime().active_tab_index;
        let Some(active_tab) = self.active_runtime().tabs.get(active_index) else {
            return;
        };
        // Self-merge: the dragged tab is the one currently showing this
        // content — there is nothing to merge into. Clear any stale hover
        // left over from hovering a different pane a moment ago.
        if active_tab.id == dragged_tab_id {
            if self.main_area.pane_drop_hover.take().is_some() {
                cx.notify();
            }
            return;
        }

        let local_x = f32::from(event.event.position.x - event.bounds.origin.x);
        let local_y = f32::from(event.event.position.y - event.bounds.origin.y);
        let w = f32::from(event.bounds.size.width);
        let h = f32::from(event.bounds.size.height);
        let hover = compute_pane_drop_hover(active_tab, local_x, local_y, w, h, None);

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
            ws.active_tab_mut()
                .map(|t| rearrange_pane(&mut t.layout, dragged, target, half))
                .unwrap_or(false)
        });
        if moved {
            self.set_focused_pane(dragged, window, cx);
            self.focus_pane(dragged, window, cx);
            self.resize_all_tabs(window, cx);
        }
        // Notify even on a no-op move: take() above cleared pane_drop_hover,
        // so the overlay must be removed from screen regardless.
        cx.notify();
    }

    /// Called from a pane's `on_drop` for a `TabDrag` payload. Mirrors
    /// `drop_pane_onto`'s zoom-guard-then-take-hover-then-act shape, but the
    /// act step merges a whole tab instead of rearranging a single leaf and
    /// is the single commit point for it (`merge_tab_into_pane` does not
    /// persist). Every exit settles the drag through `finish_tab_drag`.
    pub(in crate::workspace) fn drop_tab_onto_pane(
        &mut self,
        dragged_tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Keyboard zoom can fire between the last on_drag_move and this drop.
        // Mirror the guard in update_tab_merge_hover_from_move: never merge
        // into the full tree while a synthetic single-leaf zoom layout is
        // showing.
        if self.main_area.zoomed_pane_id.is_some() {
            self.main_area.pane_drop_hover = None;
            self.finish_tab_drag(false, cx);
            cx.notify();
            return;
        }
        let Some((target, half)) = self.main_area.pane_drop_hover.take() else {
            self.finish_tab_drag(false, cx);
            cx.notify();
            return;
        };
        let merged = self.mutate_durable_in(window, cx, |ws, window, cx| {
            ws.merge_tab_into_pane(
                dragged_tab_id,
                target,
                half.direction(),
                half.before(),
                window,
                cx,
            )
        });
        self.finish_tab_drag(merged, cx);
        cx.notify();
    }

    /// Clear a stale drop overlay from every drag-termination path that
    /// isn't a drop itself: the root `on_mouse_up`, Escape, and the root
    /// `on_mouse_move` fallback for a release outside the window.
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
