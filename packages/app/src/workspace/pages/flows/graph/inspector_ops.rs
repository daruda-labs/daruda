//! Inspector commands at the GPUI boundary: save, revert, canvas edits, and
//! the notice they leave. Each syncs the widgets first, asks the model, and
//! carries out the effects it returns.
use super::{
    FlowGraphView,
    form_bridge::FormStatus,
    inspector::{Notice, SavePlan},
};
use daruda_flow::NodeId;
use gpui::{App, Context, Window};

impl FlowGraphView {
    pub(in crate::workspace) fn has_unsaved_form(&self, cx: &App) -> bool {
        self.inspector.status().unsaved || self.name_is_dirty(cx)
    }

    pub(in crate::workspace) fn form_status(&self, cx: &App) -> FormStatus {
        let mut status = self.inspector.status();
        status.unsaved |= self.name_is_dirty(cx);
        status
    }

    #[cfg(test)]
    pub(in crate::workspace) fn draft_conflict(&self) -> bool {
        self.inspector.status().conflict
    }

    /// The header banner's sentence. A discarded-drafts notice has its own
    /// dismissible warning, so it says nothing here.
    pub(super) fn notice_message(&self) -> Option<String> {
        use crate::surface::strings::flow as s;
        Some(match self.inspector.notice()? {
            Notice::Conflict => s::form_draft_conflict(),
            Notice::Refused { message, .. } | Notice::Io(message) => message.clone(),
            Notice::InvalidDraft(node) => s::form_invalid_draft(node),
            Notice::NameEmpty => s::name_empty(),
            Notice::PromptStale => s::form_file_picker_conflict(),
            Notice::PromptOutside => s::form_file_outside(),
            Notice::Discarded(_) => return None,
        })
    }

    #[cfg(test)]
    pub(in crate::workspace) fn save_error_for_test(&self) -> Option<String> {
        self.notice_message()
    }

    pub(in crate::workspace) fn discarded_drafts(&self) -> &[NodeId] {
        match self.inspector.notice() {
            Some(Notice::Discarded(ids)) => ids,
            _ => &[],
        }
    }

    pub(in crate::workspace) fn dismiss_discarded_drafts(&mut self, cx: &mut Context<Self>) {
        self.update_inspector(|model| model.dismiss_discarded(), cx);
    }

    /// What Save would write, or `None` after showing why it cannot.
    pub(in crate::workspace) fn prepare_form_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<SavePlan> {
        self.sync_form(cx);
        match self.inspector.save_plan() {
            Ok(plan) => Some(plan),
            Err(block) => {
                let effects = self.inspector.save_blocked(block);
                self.apply_inspector_effects(effects, window, cx);
                None
            }
        }
    }

    /// Cleared before the attempt, so what is shown is about the latest one.
    pub(in crate::workspace) fn begin_form_save(&mut self, cx: &mut Context<Self>) {
        self.update_inspector(|model| model.set_notice(None), cx);
    }

    pub(in crate::workspace) fn save_io_failed(&mut self, message: String, cx: &mut Context<Self>) {
        self.update_inspector(|model| model.set_notice(Some(Notice::Io(message))), cx);
    }

    /// Put a refused save on the pane. Save writes every draft, so when the
    /// issues name another drafted node rather than the selected one, the model
    /// selects that node and its boxes say why.
    pub(in crate::workspace) fn show_save_refusal(
        &mut self,
        message: String,
        issues: &[daruda_flow::error::ValidationIssue],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_form(cx);
        let effects = self.inspector.save_refused(message, issues);
        self.apply_inspector_effects(effects, window, cx);
    }

    pub(in crate::workspace) fn finish_form_save(
        &mut self,
        selected: Option<NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let effects = self.inspector.save_succeeded(selected);
        self.apply_inspector_effects(effects, window, cx);
    }

    /// Rebase drafts onto an edit this pane wrote — a deletion, connect, or
    /// disconnect — so the pane's own change does not read as a conflict.
    pub(in crate::workspace) fn finish_canvas_edit(
        &mut self,
        deleted: &[NodeId],
        before: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_form(cx);
        let effects = self.inspector.canvas_edit(deleted, before);
        if !effects.rebind {
            self.project_form(window, cx);
        }
        self.apply_inspector_effects(effects, window, cx);
    }

    /// Discard every pending edit — the title and all node drafts — and build
    /// the selected node's form again from the file. The header's Revert.
    pub(in crate::workspace) fn rebuild_form(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let effects = self.inspector.revert_all();
        self.name_input.update(cx, |input, cx| {
            input.set_value(self.saved_name.clone(), window, cx)
        });
        self.apply_inspector_effects(effects, window, cx);
    }

    /// Build only the selected node's form again from the file. The inspector
    /// footer's Revert: other nodes' drafts and the title are not its to drop.
    pub(in crate::workspace) fn revert_selected_node(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_form(cx);
        let effects = self.inspector.revert_node();
        self.apply_inspector_effects(effects, window, cx);
    }
}

#[cfg(test)]
mod tests {
    // Commands drive widgets and the workspace; see `workspace::tests::flow`.
    // Their model halves are covered in `inspector::tests`.
}
