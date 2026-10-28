//! The bridge between the pane and the node inspector.
//!
//! The form itself is [`super::form`]'s; this is only what the pane does to it —
//! build it, put a refusal on it, and say so when a reload took away something
//! that had been typed.

use gpui::{App, Context, Window};

use super::{FlowGraphView, form};

pub(in crate::workspace) struct FormStatus {
    pub unsaved: bool,
    pub conflict: bool,
    pub node_revertable: bool,
}

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
        self._agent_watch = self.form.as_ref().map(|form| {
            cx.subscribe_in(
                &form.agent_states().id,
                window,
                |view, _, event, window, cx| {
                    if matches!(event, crate::ui::InputEvent::Change) {
                        view.refresh_form_choices(window, cx);
                    }
                },
            )
        });
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
            self._link_watches.push(cx.subscribe_in(
                form.deps_state(),
                window,
                |view, _, event, window, cx| {
                    if matches!(event, crate::ui::InputEvent::Change) {
                        view.refresh_form_links(window, cx);
                    }
                },
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

    pub(in crate::workspace) fn set_save_error(
        &mut self,
        message: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.save_error = message;
        cx.notify();
    }

    #[cfg(test)]
    pub(in crate::workspace) fn name_input_for_test(&self) -> gpui::Entity<crate::ui::InputState> {
        self.name_input.clone()
    }

    #[cfg(test)]
    pub(in crate::workspace) fn save_error_for_test(&self) -> Option<&str> {
        self.save_error.as_deref()
    }

    /// The form for the selected node, when there is one.
    pub(in crate::workspace) fn form(&self) -> Option<&form::NodeForm> {
        self.form.as_ref()
    }

    /// Pending title or inspector edits gate Run and the close confirmation.
    pub(in crate::workspace) fn has_unsaved_form(&self, cx: &App) -> bool {
        let local = self
            .form
            .as_ref()
            .is_some_and(|form| form.has_local_input(cx));
        self.unsaved_with(local, cx)
    }

    fn unsaved_with(&self, local: bool, cx: &App) -> bool {
        local || self.name_is_dirty(cx) || !self.drafts.is_empty()
    }

    /// What the Save and Revert buttons need, read once per frame. A form
    /// built from an older file can be reverted even with its boxes back at
    /// their original values: the conflict banner asks for exactly that.
    pub(in crate::workspace) fn form_status(&self, cx: &App) -> FormStatus {
        let local = self
            .form
            .as_ref()
            .is_some_and(|form| form.has_local_input(cx));
        let stale = self.form.as_ref().is_some_and(|form| {
            self.text
                .as_deref()
                .is_none_or(|text| !form.source_matches(text))
        });
        FormStatus {
            unsaved: self.unsaved_with(local, cx),
            conflict: self.draft_conflict(),
            node_revertable: local || stale,
        }
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
    pub(in crate::workspace) fn set_form_refusal(
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
        let has_notes =
            |node: &daruda_flow::NodeId| !form::notes::notes_for(issues, node).is_empty();
        let selected = self.form.as_ref().map(|form| form.node.clone());
        if !selected.as_ref().is_some_and(has_notes)
            && let Some(node) = self.drafts.keys().find(|node| has_notes(node)).cloned()
        {
            self.select_node(&node, window, cx);
        }
        let notes = self
            .form
            .as_ref()
            .map(|form| form::notes::notes_for(issues, &form.node))
            .unwrap_or_default();
        self.save_error = Some(message.clone());
        self.set_form_refusal(Some(message), notes, cx);
    }

    /// Discard every pending edit — the title and all node drafts — and build
    /// the selected node's form again from the file. The header's Revert.
    pub(in crate::workspace) fn rebuild_form(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drafts.clear();
        self.discarded.clear();
        self.name_input.update(cx, |input, cx| {
            input.set_value(self.saved_name.clone(), window, cx)
        });
        self.revert_selected_node(window, cx);
    }

    /// Build only the selected node's form again from the file. The inspector
    /// footer's Revert: other nodes' drafts and the title are not its to drop.
    pub(in crate::workspace) fn revert_selected_node(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(node) = self.form.as_ref().map(|form| form.node.clone()) {
            let text = self.text.clone();
            let issues = self.current_issues();
            self.form =
                text.and_then(|text| form::NodeForm::build(&text, &node, &issues, window, cx));
            self.watch_form(window, cx);
        }
        if !self.draft_conflict() {
            self.save_error = None;
        }
        cx.notify();
    }
}
