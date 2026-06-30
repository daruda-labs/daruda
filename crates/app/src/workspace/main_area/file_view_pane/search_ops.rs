//! Workspace-side ops for the file-viewer search panel. The View
//! captures the per-pane input entity at render time and passes it
//! back through these methods; the methods own the state transitions
//! (clearing the input, closing the panel, restoring focus).

use gpui::{Context, Entity, Window};

use crate::ui::InputState;
use crate::workspace::Workspace;

impl Workspace {
    pub(in crate::workspace) fn clear_file_view_search(
        &mut self,
        input: Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        input.update(cx, |inp, cx_state| {
            inp.set_value("", window, cx_state);
        });
        cx.notify();
    }

    pub(in crate::workspace) fn close_file_view_search(
        &mut self,
        input: Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(fv) = self.focused_file_view_mut() {
            fv.search_close();
        }
        input.update(cx, |inp, cx_state| {
            inp.set_value("", window, cx_state);
        });
        let pane_id = self.active_runtime().focused_pane_id;
        self.focus_pane(pane_id, window, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn file_view_search_prev(&mut self, cx: &mut Context<Self>) {
        if let Some(fv) = self.focused_file_view_mut() {
            fv.search_prev_match();
        }
        self.scroll_file_viewer_to_focused_match();
        cx.notify();
    }

    pub(in crate::workspace) fn file_view_search_next(&mut self, cx: &mut Context<Self>) {
        if let Some(fv) = self.focused_file_view_mut() {
            fv.search_next_match();
        }
        self.scroll_file_viewer_to_focused_match();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    // Behavioral coverage requires a live `Window`, `Context<Workspace>`,
    // and a stub input/file-view fixture. The branch logic is trivial
    // (early-return if no focused file view) so the value of a unit
    // test is low here.
}
