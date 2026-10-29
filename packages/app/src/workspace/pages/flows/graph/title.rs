//! The flow's title box: what the file is called, and an edit to it.
use gpui::{App, Context, Window};

use super::FlowGraphView;

impl FlowGraphView {
    pub(in crate::workspace) fn name(&self) -> &str {
        &self.saved_name
    }

    pub(in crate::workspace) fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub(in crate::workspace) fn edited_name(&self, cx: &App) -> String {
        self.name_input.read(cx).value().trim().to_owned()
    }

    pub(in crate::workspace) fn name_is_dirty(&self, cx: &App) -> bool {
        self.edited_name(cx) != self.saved_name
    }

    pub(in crate::workspace) fn focus_name(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.name_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub(in crate::workspace) fn name_empty(&mut self, cx: &mut Context<Self>) {
        self.inspector
            .set_notice(Some(super::inspector::Notice::NameEmpty));
        cx.notify();
    }

    #[cfg(test)]
    pub(in crate::workspace) fn name_input_for_test(&self) -> gpui::Entity<crate::ui::InputState> {
        self.name_input.clone()
    }
}

#[cfg(test)]
mod tests {
    // The title round-trip is a reload and save story; see
    // `workspace::tests::flow` for the pane-level coverage.
}
