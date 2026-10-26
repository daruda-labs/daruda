//! Event operations dispatched by the workspace frame and tab strip.

use gpui::{
    ClipboardItem, Context, Focusable as _, KeyDownEvent, MouseMoveEvent, MouseUpEvent, Window,
};

use super::layout::DockPosition;
use super::main_area::pane_tree::{PaneId, SplitDirection};
use super::main_area::tab_ops::NewPaneKind;
use super::{FileViewerSearchOpen, Workspace};

impl Workspace {
    pub(in crate::workspace) fn on_file_view_search_open(
        &mut self,
        _: &FileViewerSearchOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(fv) = self.focused_file_view_mut() {
            fv.search_open();
        }
        if let Some(fc) = self.focused_file_content() {
            let fh = fc.search_input.read(cx).focus_handle(cx);
            fh.focus(window, cx);
            cx.notify();
        }
    }

    pub(in crate::workspace) fn on_file_view_key_down(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let search_open = self
            .focused_file_view()
            .is_some_and(|fv| fv.search.is_some());
        match ev.keystroke.key.as_str() {
            // Escape while the search panel is open closes it +
            // clears the query and restores pane focus.
            // `gpui_component::Input` doesn't emit Escape via
            // `InputEvent`, so the per-pane subscription can't
            // see it; the panel-level handler picks it up.
            "escape" if search_open => {
                let pane_id = self.active_runtime().focused_pane_id;
                if let Some(fc) = self.focused_file_content() {
                    let input = fc.search_input.clone();
                    input.update(cx, |inp, cx_state| inp.set_value("", window, cx_state));
                }
                if let Some(fv) = self.focused_file_view_mut() {
                    fv.search_close();
                }
                self.focus_pane(pane_id, window, cx);
                cx.notify();
                cx.stop_propagation();
            }
            "escape" => {
                self.close_focused_file_pane(window, cx);
            }
            "c" if ev.keystroke.modifiers.platform && !search_open => {
                let text = self
                    .focused_file_view()
                    .map(|fv| fv.selected_text_for_copy())
                    .unwrap_or_default();
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            "a" if ev.keystroke.modifiers.platform && !search_open => {
                if let Some(fv) = self.focused_file_view_mut()
                    && fv.select_all()
                {
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    pub(in crate::workspace) fn on_frame_mouse_move(
        &mut self,
        ev: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A mouse-up released outside the window never reaches the
        // bubble-phase on_mouse_up below — it fails the root div's
        // hit-test — so any drag begun inside stays "live". The first
        // re-entry move carries pressed_button: None: treat that as
        // the missed release and settle every live drag (dock/divider
        // resize + file-view text/block selection) instead of
        // continuing it. The root move handler spans the whole window,
        // so it catches the release wherever the cursor re-enters.
        if !ev.dragging() {
            self.end_stale_resize_drags(cx);
            self.end_file_selection_drag(cx);
            self.clear_pane_drop_hover(cx);
            self.finish_tab_drag(false, cx);
            return;
        }
        if let Some(drag) = self.docks.drag {
            let cursor_px: f32 = match drag.position {
                DockPosition::Left | DockPosition::Right => ev.position.x.into(),
                DockPosition::Bottom => ev.position.y.into(),
            };
            self.update_dock_drag(cursor_px, window, cx);
            return;
        }
        let Some(drag) = self.main_area.drag_state else {
            return;
        };
        let cursor_px: f32 = match drag.direction {
            SplitDirection::Horizontal => ev.position.x.into(),
            SplitDirection::Vertical => ev.position.y.into(),
        };
        self.update_divider_drag(cursor_px, window, cx);
    }

    pub(in crate::workspace) fn on_frame_left_mouse_up(
        &mut self,
        _: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.end_divider_drag(cx);
        self.end_dock_drag(cx);
        self.end_file_selection_drag(cx);
        self.clear_pane_drop_hover(cx);
        self.finish_tab_drag(false, cx);
    }

    pub(in crate::workspace) fn split_tab_from_menu(
        &mut self,
        index: usize,
        kind: NewPaneKind,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable_in(window, cx, |ws, window, cx| {
            ws.split_from_tab(index, kind, direction, window, cx);
        });
    }

    pub(in crate::workspace) fn detach_pane_onto_tab_bar(
        &mut self,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable_in(window, cx, |ws, window, cx| {
            let at = ws.active_runtime().tabs.len();
            ws.detach_pane_to_new_tab(pane, at, window, cx);
        });
    }
}

#[cfg(test)]
mod tests;
