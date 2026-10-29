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

    /// The title box changed. Only an edit — not a programmatic reset to the
    /// saved title — is a reason to drop the notice it may have answered.
    pub(super) fn title_changed(&mut self, cx: &mut Context<Self>) {
        if self.name_is_dirty(cx) {
            self.update_inspector(|model| model.title_edited(), cx);
        }
    }

    pub(in crate::workspace) fn name_empty(&mut self, cx: &mut Context<Self>) {
        self.update_inspector(
            |model| model.set_notice(Some(super::inspector::Notice::NameEmpty)),
            cx,
        );
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
