//! Dock toggle handlers and divider/dock resize drag state.
//!
//! Pulls the GPUI mouse-event plumbing out of `mod.rs` so the
//! workspace entity stays focused on tabs / panes / docks /
//! lanes. Drag state (`DividerDrag`, `DockDrag`) lives here
//! because it is exclusively read by these methods plus `render.rs`.

use gpui::{Context, Pixels, Point, Window};

use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::{
    PaneId, SplitDirection, adjust_divider, find_divider, parent_axis_extent,
};
use crate::workspace::{ToggleBottomDock, ToggleLeftDock, ToggleRightDock};

use super::DockPosition;

#[derive(Clone, Copy, Debug)]
pub(in crate::workspace) struct DividerDrag {
    pub(in crate::workspace) left_first_leaf: PaneId,
    pub(in crate::workspace) direction: SplitDirection,
    pub(in crate::workspace) axis_size_px: f32,
    pub(in crate::workspace) anchor_px: f32,
    pub(in crate::workspace) start_left_ratio: f32,
}

/// Active right-click context menu. The `items` vec is produced at the
/// call site (lane row, files row, …) so the anchor carries a
/// ready-to-render item list. Cleared by `close_context_menu` or when a
/// backdrop click is received.
pub(in crate::workspace) struct ContextMenuAnchor {
    pub(in crate::workspace) position: Point<Pixels>,
    pub(in crate::workspace) items: Vec<crate::ui::ContextMenuItem>,
    /// Which corner of the menu lands on `position`. `TopLeft` is the
    /// right-click default; `BottomRight` is used by chips near a
    /// container edge so the menu expands inward instead of clipping.
    pub(in crate::workspace) corner: crate::ui::ContextMenuCorner,
}

/// Drag state for a dock resize handle. `anchor_px` is the cursor
/// coordinate at mousedown (x for Left/Right, y for Bottom); deltas
/// from that anchor translate into `Dock::resize()` calls.
#[derive(Clone, Copy, Debug)]
pub(in crate::workspace) struct DockDrag {
    pub(in crate::workspace) position: DockPosition,
    pub(in crate::workspace) anchor_px: f32,
    pub(in crate::workspace) start_size: f32,
}

impl Workspace {
    pub(in crate::workspace) fn on_toggle_left_dock(
        &mut self,
        _: &ToggleLeftDock,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable(cx, |ws, cx| {
            ws.left_dock.update(cx, |d, _| d.toggle());
            ws.main_area.pending_resize = true;
        });
        cx.notify();
    }

    pub(in crate::workspace) fn on_toggle_bottom_dock(
        &mut self,
        _: &ToggleBottomDock,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable(cx, |ws, cx| {
            // Notify the Dock entity, not just the Workspace: the bottom
            // dock is rendered through `.cached()`, so a Workspace-only
            // notify would leave the cached (closed) view on screen.
            // Per root CLAUDE.md Pitfall #10.
            ws.bottom_dock.update(cx, |d, cx| {
                d.toggle();
                cx.notify();
            });
            ws.main_area.pending_resize = true;
        });
        cx.notify();
    }

    pub(in crate::workspace) fn on_toggle_right_dock(
        &mut self,
        _: &ToggleRightDock,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable(cx, |ws, cx| {
            ws.right_dock.update(cx, |d, _| d.toggle());
            ws.main_area.pending_resize = true;
        });
        cx.notify();
    }

    // ---- Divider drag ----

    pub(in crate::workspace) fn begin_divider_drag(
        &mut self,
        left_first_leaf: PaneId,
        anchor_px: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.main_area.tabs.get(self.main_area.active_tab_index) else {
            return;
        };
        let Some((direction, start_left_ratio)) = find_divider(&tab.layout, left_first_leaf) else {
            return;
        };
        let (w, h) = self.main_area.last_viewport.unwrap_or((1.0, 1.0));
        let Some(axis_size_px) = parent_axis_extent(&tab.layout, w, h, left_first_leaf) else {
            return;
        };
        self.main_area.drag_state = Some(DividerDrag {
            left_first_leaf,
            direction,
            axis_size_px,
            anchor_px,
            start_left_ratio,
        });
        cx.notify();
    }

    pub(in crate::workspace) fn update_divider_drag(
        &mut self,
        cursor_px: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.main_area.drag_state else {
            return;
        };
        if drag.axis_size_px <= f32::EPSILON {
            return;
        }
        let delta_px = cursor_px - drag.anchor_px;
        let delta_ratio = delta_px / drag.axis_size_px;
        if let Some(tab) = self.main_area.tabs.get_mut(self.main_area.active_tab_index) {
            // Reset ratio to start before applying delta to avoid drift.
            // adjust_divider applies relative delta from current ratios, so
            // reapply (target - current) to converge on the desired value.
            if let Some((_, current_left)) = find_divider(&tab.layout, drag.left_first_leaf) {
                let target_left = drag.start_left_ratio + delta_ratio;
                let apply = target_left - current_left;
                adjust_divider(&mut tab.layout, drag.left_first_leaf, apply);
            }
        }
        self.resize_all_tabs(window, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn end_divider_drag(&mut self, cx: &mut Context<Self>) {
        if self.main_area.drag_state.is_some() {
            self.mutate_durable(cx, |ws, _| {
                ws.main_area.drag_state = None;
            });
            cx.notify();
        }
    }

    // ---- Dock resize drag ----

    pub(in crate::workspace) fn begin_dock_drag(
        &mut self,
        position: DockPosition,
        anchor_px: f32,
        cx: &mut Context<Self>,
    ) {
        let start_size = match position {
            DockPosition::Left => self.left_dock.read(cx).size,
            DockPosition::Right => self.right_dock.read(cx).size,
            DockPosition::Bottom => self.bottom_dock.read(cx).size,
        };
        self.dock_drag = Some(DockDrag {
            position,
            anchor_px,
            start_size,
        });
        cx.notify();
    }

    pub(in crate::workspace) fn update_dock_drag(
        &mut self,
        cursor_px: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.dock_drag else {
            return;
        };
        let delta = cursor_px - drag.anchor_px;
        // Sign convention: cursor moving away from the center pane
        // grows the dock. Left dock sits on the left of center, so a
        // rightward drag (positive delta) grows it; right and bottom
        // docks sit on the opposite side, so we negate.
        let new_size = match drag.position {
            DockPosition::Left => drag.start_size + delta,
            DockPosition::Right => drag.start_size - delta,
            DockPosition::Bottom => drag.start_size - delta,
        };
        match drag.position {
            DockPosition::Left => self.left_dock.update(cx, |d, _| d.resize(new_size)),
            DockPosition::Right => self.right_dock.update(cx, |d, _| d.resize(new_size)),
            // Bottom dock renders through `.cached()`, so notify its
            // entity directly (Pitfall #10) — a Workspace-only notify
            // wouldn't repaint the cached view at the new height.
            DockPosition::Bottom => self.bottom_dock.update(cx, |d, cx| {
                d.resize(new_size);
                cx.notify();
            }),
        };
        self.resize_all_tabs(window, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn end_dock_drag(&mut self, cx: &mut Context<Self>) {
        if self.dock_drag.is_some() {
            self.mutate_durable(cx, |ws, _| {
                ws.dock_drag = None;
            });
            cx.notify();
        }
    }

    /// End any live dock/divider resize drag. Called when a move event
    /// arrives with the primary button no longer held — the release that
    /// should have ended the drag happened outside the window, where the
    /// bubble-phase `on_mouse_up` never fires (it fails the root hit-test).
    /// Without this, the stale drag keeps resizing on the next in-window move.
    pub(in crate::workspace) fn end_stale_resize_drags(&mut self, cx: &mut Context<Self>) {
        self.end_dock_drag(cx);
        self.end_divider_drag(cx);
    }

    /// Resize the bottom dock to a preset height matching N rows of
    /// macro tiles. Wires through the same path as a drag-release:
    /// `Dock::resize` (clamps to min/max), `resize_all_tabs` so PTYs
    /// pick up the new viewport, and `mark_dirty_and_save` so the new
    /// size persists across restarts.
    pub(in crate::workspace) fn set_bottom_dock_row_preset(
        &mut self,
        new_size: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable_in(window, cx, |ws, window, cx| {
            // Cached bottom dock — notify its entity (Pitfall #10).
            ws.bottom_dock.update(cx, |d, cx| {
                d.resize(new_size);
                cx.notify();
            });
            ws.resize_all_tabs(window, cx);
        });
        cx.notify();
    }

    // ---- Context menu ----

    pub(in crate::workspace) fn open_context_menu(
        &mut self,
        position: Point<Pixels>,
        items: Vec<crate::ui::ContextMenuItem>,
        cx: &mut Context<Self>,
    ) {
        self.open_context_menu_at_corner(
            position,
            items,
            crate::ui::ContextMenuCorner::TopLeft,
            cx,
        );
    }

    /// Open the context menu with a custom anchor corner. Use this
    /// when the click target sits near a container edge and the
    /// default `TopLeft` would clip the menu — `BottomRight` flips the
    /// expansion direction so the menu opens up-and-left from
    /// `position` instead of down-and-right.
    pub(in crate::workspace) fn open_context_menu_at_corner(
        &mut self,
        position: Point<Pixels>,
        items: Vec<crate::ui::ContextMenuItem>,
        corner: crate::ui::ContextMenuCorner,
        cx: &mut Context<Self>,
    ) {
        self.main_area.context_menu = Some(ContextMenuAnchor {
            position,
            items,
            corner,
        });
        cx.notify();
    }

    pub(in crate::workspace) fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.main_area.context_menu.is_some() {
            self.main_area.context_menu = None;
            cx.notify();
        }
    }
}
