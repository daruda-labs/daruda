//! The bridge between the pane and the node inspector.
//!
//! The form itself is [`super::form`]'s; this is only what the pane does to it —
//! build it, put a refusal on it, and say so when a reload took away something
//! that had been typed.

use gpui::{App, Context, Window};

use super::{
    FlowGraphView, form,
    inspector::{InspectorStatus, Notice},
};

pub(in crate::workspace) type FormStatus = InspectorStatus;

impl FlowGraphView {
    pub(in crate::workspace) fn set_catalog(
        &mut self,
        catalog: form::catalog::Catalog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.catalog = catalog;
        self.refresh_form_choices(window, cx);
    }

    pub(super) fn refresh_form_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.form {
            form.refresh_choices(&self.catalog, window, cx);
        }
        cx.notify();
    }

    pub(super) fn watch_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._form_watches.clear();
        if let Some(form) = &self.form {
            let guard = form.applying();
            let node = form.node.clone();
            for input in form.watch_inputs() {
                let guard = guard.clone();
                let node = node.clone();
                self._form_watches.push(cx.subscribe_in(
                    &input,
                    window,
                    move |view, _, event, window, cx| {
                        view.on_bound_input(&guard, &node, event, window, cx)
                    },
                ));
            }
            let selects = form.watch_selects(cx);
            for select in selects {
                let guard = guard.clone();
                let node = node.clone();
                self._form_watches.push(cx.subscribe_in(
                    &select,
                    window,
                    move |view, _, _: &crate::ui::select::ConfirmEvent, window, cx| {
                        view.on_bound_select(&guard, &node, window, cx)
                    },
                ));
            }
        }
        self._link_watches.clear();
        if let Some(form) = &self.form {
            self._link_watches.push(cx.subscribe_in(
                &form.dep_picker,
                window,
                |view, _, event, window, cx| view.choose_form_link(false, event, window, cx),
            ));
            self._link_watches.push(cx.subscribe_in(
                &form.result_picker,
                window,
                |view, _, event, window, cx| view.choose_form_link(true, event, window, cx),
            ));
        }
        self.watch_result_targets(window, cx);
        self.refresh_form_choices(window, cx);
        self.refresh_form_links(window, cx);
    }

    pub(super) fn toggle_execution_section(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.form {
            form.settings_open = !form.settings_open;
            cx.notify();
        }
    }

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

    pub(in crate::workspace) fn save_io_failed(&mut self, message: String, cx: &mut Context<Self>) {
        self.inspector.set_notice(Some(Notice::Io(message)));
        cx.notify();
    }

    pub(in crate::workspace) fn begin_form_save(&mut self, cx: &mut Context<Self>) {
        self.inspector.set_notice(None);
        self.set_form_refusal(None, Vec::new(), cx);
    }

    #[cfg(test)]
    pub(in crate::workspace) fn name_input_for_test(&self) -> gpui::Entity<crate::ui::InputState> {
        self.name_input.clone()
    }

    #[cfg(test)]
    pub(in crate::workspace) fn save_error_for_test(&self) -> Option<String> {
        self.notice_message()
    }

    /// The form for the selected node, when there is one.
    #[cfg(any(test, feature = "screenshot"))]
    pub(in crate::workspace) fn form(&self) -> Option<&form::FormBinding> {
        self.form.as_ref()
    }

    pub(in crate::workspace) fn has_unsaved_form(&self, cx: &App) -> bool {
        self.inspector.status().unsaved || self.name_is_dirty(cx)
    }

    pub(in crate::workspace) fn form_status(&self, cx: &App) -> FormStatus {
        let mut status = self.inspector.status();
        status.unsaved |= self.name_is_dirty(cx);
        status
    }

    fn on_bound_input(
        &mut self,
        guard: &std::cell::Cell<usize>,
        node: &daruda_flow::NodeId,
        event: &crate::ui::InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, crate::ui::InputEvent::Change) {
            self.on_bound_select(guard, node, window, cx);
        }
    }

    fn on_bound_select(
        &mut self,
        guard: &std::cell::Cell<usize>,
        node: &daruda_flow::NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if guard.get() == 0 && self.inspector.selected() == Some(node) {
            self.on_form_changed(window, cx);
        }
    }

    pub(super) fn on_form_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let effects = self.sync_form(cx);
        self.apply_inspector_effects(effects, window, cx);
    }

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

    pub(in crate::workspace) fn name_empty(&mut self, cx: &mut Context<Self>) {
        self.inspector.set_notice(Some(Notice::NameEmpty));
        cx.notify();
    }

    /// Open or close the inspector's agent-override block.
    pub(in crate::workspace) fn toggle_agent_section(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.form.as_mut() {
            form.toggle_agent_open();
            cx.notify();
        }
    }

    /// Put a refused save's reason on the form — the sentence, and the boxes it
    /// names — or clear both.
    fn set_form_refusal(
        &mut self,
        message: Option<String>,
        notes: Vec<form::notes::FieldNote>,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.form.as_mut() {
            form.set_banner(message);
            form.set_notes(notes);
            cx.notify();
        }
    }

    /// Put a refused save on the pane. Save writes every draft, so when the
    /// issues name another drafted node rather than the selected one, that node
    /// is selected and its boxes say why.
    pub(in crate::workspace) fn show_save_refusal(
        &mut self,
        message: String,
        issues: &[daruda_flow::error::ValidationIssue],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_form(cx);
        let effects = self.inspector.save_refused(message.clone(), issues);
        self.apply_inspector_effects(effects, window, cx);
        let issues = match self.inspector.notice() {
            Some(Notice::Refused { issues, .. }) => issues.as_slice(),
            _ => &[],
        };
        let notes = self
            .inspector
            .draft()
            .map(|draft| {
                let notes = form::notes::notes_for(issues, draft.node());
                if notes.is_empty() {
                    form::notes::notes_for(
                        issues,
                        &daruda_flow::NodeId::from(draft.raw().id.trim()),
                    )
                } else {
                    notes
                }
            })
            .unwrap_or_default();
        self.set_form_refusal(Some(message), notes, cx);
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
