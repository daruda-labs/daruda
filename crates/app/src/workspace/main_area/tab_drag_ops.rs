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
//! `PaneHeaderDrag`): each cell's `on_drag_move` carries its own
//! GPUI-computed bounds, so nothing has to re-derive the tab bar's flex
//! layout. GPUI fires those listeners wherever the cursor is, though, so a
//! cell classifies the cursor against its own bounds (`cell_drag_action`)
//! and only ever touches state it owns — every cell runs on every mouse
//! move, in no guaranteed order.

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

/// What one tab cell's `on_drag_move` should do for a single mouse move.
#[derive(Debug, PartialEq, Eq)]
enum CellDragAction {
    /// Cursor is over this cell — preview an insertion at this slot.
    Insert(usize),
    /// Cursor is on the tab row but not over this cell — a neighbour or the
    /// end slot owns the drag; leave the shared state untouched.
    Hold,
    /// Cursor left the tab row — release the state this cell owns.
    Release,
}

/// Pure hit-test behind `update_tab_drag_from_move`. Inside the cell → the
/// reorder slot the midpoint implies (west half → before, east half →
/// after). The two outside cases differ by axis, and that split is what
/// keeps the "+" button's insert-at-end drop working: cells sit in one row,
/// so y decides whether the cursor is still on the tab row (`Hold` — some
/// other slot owns the drag, including the end slot past the last cell) or
/// has left it (`Release`).
///
/// The row is measured against `row_height`, not the cell: `items_center`
/// centres a shorter cell in a taller bar, and the leftover band is still
/// tab row. Testing y falls to this function at all because GPUI's
/// `on_drag_move`, unlike `on_mouse_move`, has no `hitbox.is_hovered` guard
/// (gpui `div.rs`): every cell's listener runs wherever the cursor is.
fn cell_drag_action(
    local_x: f32,
    local_y: f32,
    cell_width: f32,
    cell_height: f32,
    row_height: f32,
    cell_index: usize,
) -> CellDragAction {
    let overhang = ((row_height - cell_height) / 2.0).max(0.0);
    if !(-overhang..=cell_height + overhang).contains(&local_y) {
        return CellDragAction::Release;
    }
    if !(0.0..=cell_width).contains(&local_x) {
        return CellDragAction::Hold;
    }
    CellDragAction::Insert(if local_x < cell_width / 2.0 {
        cell_index
    } else {
        cell_index + 1
    })
}

impl Workspace {
    /// Called from each tab cell's own `on_drag_move`. The cell classifies
    /// the cursor itself: claim the drag while over it, stand aside while
    /// another slot holds it, release its own state once the cursor leaves
    /// the tab row.
    pub(in crate::workspace) fn update_tab_drag_from_move(
        &mut self,
        cell_tab_id: u64,
        cell_index: usize,
        event: &DragMoveEvent<TabDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let local_x = f32::from(event.event.position.x - event.bounds.origin.x);
        let local_y = f32::from(event.event.position.y - event.bounds.origin.y);
        let w = f32::from(event.bounds.size.width);
        let h = f32::from(event.bounds.size.height);
        match cell_drag_action(local_x, local_y, w, h, theme::TAB_BAR_HEIGHT, cell_index) {
            CellDragAction::Insert(target_index) => {
                if self.main_area.tab_reorder_preview != Some((cell_tab_id, target_index)) {
                    self.main_area.tab_reorder_preview = Some((cell_tab_id, target_index));
                    cx.notify();
                }
                self.arm_tab_hover_switch(cell_tab_id, window, cx);
            }
            CellDragAction::Hold => {}
            CellDragAction::Release => self.release_tab_drag_state_owned_by(cell_tab_id, cx),
        }
    }

    /// Drop the reorder preview and hover-switch countdown *this* cell set,
    /// leaving another cell's alone. Tagging by tab id keeps a cell from
    /// clearing a slot it never owned, rather than leaning on every cell
    /// happening to leave the tab row in the same frame.
    pub(in crate::workspace) fn release_tab_drag_state_owned_by(
        &mut self,
        cell_tab_id: u64,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        if self.main_area.tab_reorder_preview.map(|(owner, _)| owner) == Some(cell_tab_id) {
            self.main_area.tab_reorder_preview = None;
            changed = true;
        }
        if self.main_area.tab_hover_switch.as_ref().map(|(id, _)| *id) == Some(cell_tab_id) {
            self.main_area.tab_hover_switch = None; // drops (cancels) the timer
            changed = true;
        }
        if changed {
            cx.notify();
        }
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

    /// Called from a tab cell's `on_drop`. Splices the tab to the live
    /// reorder-preview slot, persisted through `mutate_durable_in` (unlike
    /// the preview switch). No armed slot, an unknown tab, or a same-position
    /// landing moves nothing, so the drag ends as abandoned.
    pub(in crate::workspace) fn drop_tab_onto_bar(
        &mut self,
        dragged_tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target_index = self.main_area.tab_reorder_preview.map(|(_, index)| index);
        let from = self
            .active_runtime()
            .tabs
            .iter()
            .position(|t| t.id == dragged_tab_id);
        let committed = match (target_index, from) {
            (Some(target_index), Some(from)) => {
                let to = if target_index > from {
                    target_index - 1
                } else {
                    target_index
                };
                to != from && {
                    self.mutate_durable_in(window, cx, |ws, _window, cx| ws.move_tab(from, to, cx));
                    true
                }
            }
            _ => false,
        };
        self.finish_tab_drag(committed, cx);
        cx.notify();
    }

    /// Escape while any drag is in flight cancels it, mirroring zed's
    /// `Workspace::cancel`. GPUI ends a drag on mouse-up only, so this is
    /// the sole path that calls one off without a release.
    pub(in crate::workspace) fn cancel_active_drag(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !cx.stop_active_drag(window) {
            return false;
        }
        self.clear_pane_drop_hover(cx);
        self.finish_tab_drag(false, cx);
        true
    }

    /// Root capture-phase Escape hook. Capture runs root→leaf, ahead of every
    /// focused widget's own Escape handling, so a live drag wins the key;
    /// with no drag in flight the key travels on untouched.
    pub(in crate::workspace) fn cancel_drag_on_escape(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" && self.cancel_active_drag(window, cx) {
            cx.stop_propagation();
        }
    }

    /// The one way a tab drag ends — every termination path routes here: the
    /// two drops, the root `on_mouse_up`, Escape, and the root
    /// `on_mouse_move` fallback for a release outside the window. Without a
    /// committed drop the hover preview is unwound to the pre-drag tab, since
    /// the preview is a look-ahead and must leave no trace.
    pub(in crate::workspace) fn finish_tab_drag(
        &mut self,
        committed: bool,
        cx: &mut Context<Self>,
    ) {
        // Every `.take()` runs unconditionally: `||` would short-circuit past
        // the later fields whenever an earlier one was set (hovering a cell
        // sets two), stranding a stale indicator on screen.
        let had_hover_switch = self.main_area.tab_hover_switch.take().is_some();
        let had_reorder_preview = self.main_area.tab_reorder_preview.take().is_some();
        let restore_target = self.main_area.tab_preview_restore.take();

        let restored = match restore_target {
            Some(tab_id) if !committed => self.restore_active_tab_by_id(tab_id),
            _ => false,
        };

        if had_hover_switch || had_reorder_preview || restored {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CellDragAction, cell_drag_action};

    /// Stand-in tab-cell geometry, to scale with the real thing: a 20px-tall
    /// cell (`TAB_PAD_Y` twice plus the close button) centred by
    /// `items_center` in the 28px tab bar, so 4px of row overhangs it above
    /// and below.
    const W: f32 = 100.0;
    const H: f32 = 20.0;
    const ROW: f32 = 28.0;

    /// `items_center` leaves a few px of tab bar above and below each cell.
    /// A cursor there is still on the tab row and still in this cell's
    /// column, so it keeps targeting this cell — a `Release` would cancel
    /// the hover-switch and drop the insertion line mid-drag.
    #[test]
    fn row_overhang_above_and_below_the_cell_still_targets_it() {
        assert_eq!(
            cell_drag_action(5.0, -3.0, W, H, ROW, 2),
            CellDragAction::Insert(2)
        );
        assert_eq!(
            cell_drag_action(60.0, H + 3.0, W, H, ROW, 2),
            CellDragAction::Insert(3)
        );
    }

    /// Same overhang band, but outside this cell's column — where the "+"
    /// end slot lives. Another slot owns the drag there.
    #[test]
    fn row_overhang_outside_the_column_holds() {
        assert_eq!(
            cell_drag_action(101.0, -3.0, W, H, ROW, 2),
            CellDragAction::Hold
        );
    }

    /// Past the row's own edge is a real exit — that is the pane area.
    #[test]
    fn past_the_row_edge_releases() {
        assert_eq!(
            cell_drag_action(50.0, -5.0, W, H, ROW, 2),
            CellDragAction::Release
        );
        assert_eq!(
            cell_drag_action(50.0, H + 5.0, W, H, ROW, 2),
            CellDragAction::Release
        );
    }

    /// A cell that already fills its row has no overhang to forgive.
    #[test]
    fn a_full_height_cell_releases_immediately_outside_itself() {
        assert_eq!(
            cell_drag_action(50.0, -1.0, W, H, H, 2),
            CellDragAction::Release
        );
    }

    #[test]
    fn west_half_targets_this_cell() {
        assert_eq!(
            cell_drag_action(5.0, 10.0, W, H, ROW, 2),
            CellDragAction::Insert(2)
        );
    }

    #[test]
    fn east_half_targets_next_cell() {
        assert_eq!(
            cell_drag_action(60.0, 10.0, W, H, ROW, 2),
            CellDragAction::Insert(3)
        );
    }

    #[test]
    fn exact_midpoint_rounds_to_east() {
        assert_eq!(
            cell_drag_action(50.0, 10.0, W, H, ROW, 0),
            CellDragAction::Insert(1)
        );
    }

    /// Still on the tab row, just not over this cell — the cursor is over a
    /// neighbour or the end slot past the last tab (where the "+" button
    /// renders the insert-at-end indicator). Whoever owns the drag there
    /// keeps it; this cell must not touch the shared state.
    #[test]
    fn outside_cell_on_the_x_axis_only_holds() {
        assert_eq!(
            cell_drag_action(-1.0, 10.0, W, H, ROW, 2),
            CellDragAction::Hold
        );
        assert_eq!(
            cell_drag_action(101.0, 10.0, W, H, ROW, 2),
            CellDragAction::Hold
        );
    }

    /// Dragging down into the pane area leaves the cursor's x inside some
    /// cell's column, and GPUI calls that cell's `on_drag_move` regardless.
    /// The y check is the only thing keeping that cell from arming its
    /// hover-switch and yanking the active tab back mid-drop.
    #[test]
    fn cursor_below_the_tab_bar_releases_even_when_x_is_inside() {
        assert_eq!(
            cell_drag_action(50.0, 400.0, W, H, ROW, 2),
            CellDragAction::Release
        );
    }

    #[test]
    fn cursor_above_the_tab_bar_releases_even_when_x_is_inside() {
        assert_eq!(
            cell_drag_action(50.0, -8.0, W, H, ROW, 2),
            CellDragAction::Release
        );
    }

    #[test]
    fn boundary_edges_are_inclusive_on_both_axes() {
        assert_eq!(
            cell_drag_action(0.0, 0.0, W, H, ROW, 2),
            CellDragAction::Insert(2)
        );
        assert_eq!(
            cell_drag_action(W, H, W, H, ROW, 2),
            CellDragAction::Insert(3)
        );
    }
}
