//! Dock toggle handlers and divider/dock resize drag state.
//!
//! Pulls the GPUI mouse-event plumbing out of `mod.rs` so the
//! workspace entity stays focused on tabs / panes / docks /
//! lanes. Drag state (`DividerDrag`, `DockDrag`) lives here
//! because it is exclusively read by these methods plus `render.rs`.

use gpui::{Context, DismissEvent, Entity, Focusable as _, Pixels, Point, Subscription, Window};

use crate::ui::PopupMenu;
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

/// The open right-click menu, if any. Every menu in the app deploys here
/// and is painted at the workspace root by `crate::ui::popup_menu_deferred`
/// — see `workspace::root_menu`. `None` = closed.
pub(in crate::workspace) struct PopupMenuDeploy {
    pub(in crate::workspace) menu: Entity<PopupMenu>,
    pub(in crate::workspace) position: Point<Pixels>,
    _dismiss_sub: Subscription,
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

/// Pure height formula for the bottom dock auto-grow path.
///
/// Given a display-row count (already clamped to `max_rows`) returns the
/// desired dock height in pixels. The layout is stacked (`flex_col`):
/// text area on top (full-width) + action row below (mode chip + Submit).
///
/// Formula: `TAB_BAR_HEIGHT + 2*PANEL_BODY_PAD_Y + DOCK_BOTTOM_INPUT_TEXT_PAD_H +
///           rows * DOCK_BOTTOM_INPUT_EXTRA_LINE_H + DOCK_BOTTOM_INPUT_ACTION_ROW_H`
///
/// Each display row (soft-wrapped or hard-newline) contributes
/// `DOCK_BOTTOM_INPUT_EXTRA_LINE_H` (20 px = `Rems(1.25) × 16 px`).
/// The action row (`DOCK_BOTTOM_INPUT_ACTION_ROW_H`) is a fixed addend
/// outside the per-row loop — it is not repeated with more lines.
///
/// Extracted as a free function so it can be unit-tested without a GPUI context.
pub(in crate::workspace) fn bottom_dock_height_for_rows(rows: usize) -> f32 {
    use crate::ui::theme::{
        DOCK_BOTTOM_INPUT_ACTION_ROW_H, DOCK_BOTTOM_INPUT_EXTRA_LINE_H,
        DOCK_BOTTOM_INPUT_TEXT_PAD_H, PANEL_BODY_PAD_Y, TAB_BAR_HEIGHT,
    };
    TAB_BAR_HEIGHT
        + 2.0 * PANEL_BODY_PAD_Y
        + DOCK_BOTTOM_INPUT_TEXT_PAD_H
        + rows as f32 * DOCK_BOTTOM_INPUT_EXTRA_LINE_H
        + DOCK_BOTTOM_INPUT_ACTION_ROW_H
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

    /// Reveal `view` in the right dock: open the dock if it's closed,
    /// then select the view. `set_right_dock_view` alone only moves the
    /// tab selection and returns early when the view is already
    /// selected, so a "show me this panel" affordance built on it is a
    /// silent no-op whenever the dock is closed — and `Usage` is the
    /// default selection, so that was every first click.
    pub(in crate::workspace) fn reveal_right_dock_view(
        &mut self,
        view: daruda_store::project::RightDockView,
        cx: &mut Context<Self>,
    ) {
        if !self.right_dock.read(cx).is_open {
            self.mutate_durable(cx, |ws, cx| {
                ws.right_dock.update(cx, |d, _| d.open());
                ws.main_area.pending_resize = true;
            });
        }
        self.set_right_dock_view(view, cx);
        cx.notify();
    }

    // ---- Divider drag ----

    pub(in crate::workspace) fn begin_divider_drag(
        &mut self,
        left_first_leaf: PaneId,
        anchor_px: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .active_runtime()
            .tabs
            .get(self.active_runtime().active_tab_index)
        else {
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
        if let Some(tab) = self.active_tab_mut() {
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

    /// Recompute and apply the bottom dock height to match the current
    /// line count of the shared bottom input. Called from the
    /// `InputEvent::Change` subscription in `workspace/mod.rs`.
    ///
    /// Height formula: `DOCK_BOTTOM_ROW_PRESET_1_H` covers the 1-row
    /// base (tab bar + padding + one text line); each additional line
    /// adds `DOCK_BOTTOM_INPUT_EXTRA_LINE_H`. The result is clamped to
    /// `[DOCK_BOTTOM_MIN_H, DOCK_BOTTOM_MAX_H]` by `Dock::resize`.
    ///
    /// Redundant calls are guarded: the new height is compared to the
    /// dock's current size before going through the resize path to
    /// avoid notify storms when the content changes within the same row
    /// count (e.g. typing within a single line doesn't change N).
    pub(in crate::workspace) fn adapt_dock_to_input_lines(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Read display-row count from the input state (re-entrant-safe: we read
        // `terminal_input` here while holding `&mut Workspace` — it is a
        // separate entity so no re-entry conflict; we are not inside an
        // `entity.update` on `terminal_input`).
        //
        // `display_rows()` returns the soft-wrapped row count that the editor
        // uses for its own layout (`update_auto_grow` → `text_wrapper.len()`),
        // so the dock height stays in sync with the editor's actual height.
        // Hard-newline count (`text().lines_len()`) would clip long soft-wrapped
        // lines at the bottom.
        // ⚠️ Do NOT call `window.line_height()` / `window.text_style()` here —
        // this method runs in an event handler, outside the paint walk (CLAUDE.md
        // Pitfall #8). The per-row height constant lives in the palette instead.
        let new_line_count = self.terminal_input.read(cx).display_rows().max(1);

        let max_rows = usize::from(self.agent.input_max_rows);
        let clamped = new_line_count.min(max_rows);

        // Guard: only resize when line count actually changes.
        if clamped == self.terminal_input_line_count {
            return;
        }
        self.terminal_input_line_count = clamped;

        let desired = bottom_dock_height_for_rows(clamped);

        // Grow-only: auto-height may enlarge the dock to fit more lines, but
        // never shrinks it. The user manually dragged the dock to its current
        // height; reducing it (e.g. after clearing the input on submit) would
        // undo that intentional action. A smaller `desired` is silently
        // ignored; only when `desired > current` do we invoke the resize path.
        let current = self.bottom_dock.read(cx).size;
        if desired <= current {
            return;
        }

        self.set_bottom_dock_row_preset(desired, window, cx);
    }

    // ---- Context menu (root-deployed PopupMenu) ----

    /// Open a `PopupMenu` at `position`, rendered at the workspace root by
    /// `crate::ui::popup_menu_deferred`. Every right-click menu in the app
    /// arrives here — see `workspace::root_menu` for why none of them may
    /// attach the menu inside their own subtree. The dismiss subscription
    /// routes Escape / outside-click / confirmed-item-click back through
    /// `close_context_menu` uniformly.
    ///
    /// Focus moves to the menu: `PopupMenu` binds its arrow / confirm /
    /// dismiss actions under its own `key_context`, and those only receive
    /// dispatch along the focus path, so without this the menu is
    /// mouse-only. The vendored declarative attachment focuses on every
    /// frame it is open; doing it once on open is enough here because the
    /// deploy slot is the only thing that opens one.
    pub(in crate::workspace) fn open_context_menu(
        &mut self,
        position: Point<Pixels>,
        menu: Entity<PopupMenu>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle = menu.focus_handle(cx);
        if !handle.contains_focused(window, cx) {
            handle.focus(window, cx);
        }
        let _dismiss_sub = cx.subscribe(&menu, |this, _menu, _event: &DismissEvent, cx| {
            this.close_context_menu(cx);
        });
        self.main_area.popup_menu_deploy = Some(PopupMenuDeploy {
            menu,
            position,
            _dismiss_sub,
        });
        cx.notify();
    }

    pub(in crate::workspace) fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.main_area.popup_menu_deploy.take().is_some() {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bottom_dock_height_arithmetic() {
        use crate::ui::theme::{
            DOCK_BOTTOM_INPUT_ACTION_ROW_H, DOCK_BOTTOM_INPUT_EXTRA_LINE_H,
            DOCK_BOTTOM_INPUT_TEXT_PAD_H, PANEL_BODY_PAD_Y, TAB_BAR_HEIGHT,
        };

        // Stacked layout base (shared fixed overhead):
        //   TAB_BAR_HEIGHT + 2*PANEL_BODY_PAD_Y + DOCK_BOTTOM_INPUT_TEXT_PAD_H
        //   + DOCK_BOTTOM_INPUT_ACTION_ROW_H
        let fixed = TAB_BAR_HEIGHT
            + 2.0 * PANEL_BODY_PAD_Y
            + DOCK_BOTTOM_INPUT_TEXT_PAD_H
            + DOCK_BOTTOM_INPUT_ACTION_ROW_H;

        // 1 row — fixed overhead + 1 line.
        assert_eq!(
            bottom_dock_height_for_rows(1),
            fixed + DOCK_BOTTOM_INPUT_EXTRA_LINE_H,
            "1 row should be fixed overhead + one line step"
        );

        // 2 rows — fixed overhead + 2 line steps.
        assert_eq!(
            bottom_dock_height_for_rows(2),
            fixed + 2.0 * DOCK_BOTTOM_INPUT_EXTRA_LINE_H,
            "2 rows should be fixed overhead + two line steps"
        );

        // At max cap (8 rows by default config) — fixed + 8 line steps.
        let max_rows = 8_usize;
        assert_eq!(
            bottom_dock_height_for_rows(max_rows),
            fixed + max_rows as f32 * DOCK_BOTTOM_INPUT_EXTRA_LINE_H,
            "8 rows (max cap) should be fixed overhead + 8 line steps"
        );

        // Delta between adjacent row counts must equal exactly one extra-line step.
        assert_eq!(
            bottom_dock_height_for_rows(3) - bottom_dock_height_for_rows(2),
            DOCK_BOTTOM_INPUT_EXTRA_LINE_H,
            "each additional row adds exactly one extra-line step"
        );

        // Regression guard: extra-line step must be 20 px (Rems(1.25) × 16px),
        // not 16 px (was incorrect after a mis-attributed terminal-font reasoning).
        assert_eq!(
            DOCK_BOTTOM_INPUT_EXTRA_LINE_H, 20.0,
            "extra-line step must be 20 px (Rems(1.25) × rem_size(16 px))"
        );

        // Grow-only invariant: a smaller row count must produce a smaller or
        // equal desired height, never larger. This ensures the grow-only guard
        // in `adapt_dock_to_input_lines` correctly returns early when line
        // count drops.
        assert!(
            bottom_dock_height_for_rows(1) < bottom_dock_height_for_rows(2),
            "height must be strictly monotone: fewer rows → smaller desired"
        );
        assert!(
            bottom_dock_height_for_rows(2) < bottom_dock_height_for_rows(3),
            "height must be strictly monotone: fewer rows → smaller desired"
        );
    }
}
