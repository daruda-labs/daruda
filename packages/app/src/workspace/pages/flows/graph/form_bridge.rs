//! The bridge between the pane and the node inspector.
//!
//! The form itself is [`super::form`]'s; this is only what the pane does to it —
//! build it, put a refusal on it, and say so when a reload took away something
//! that had been typed.

use gpui::{App, Context, Window};

use super::{FlowGraphView, form};

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

    /// The form for the selected node, when there is one.
    pub(in crate::workspace) fn form(&self) -> Option<&form::NodeForm> {
        self.form.as_ref()
    }

    /// Pending title or inspector edits gate Run and the close confirmation.
    pub(in crate::workspace) fn has_unsaved_form(&self, cx: &App) -> bool {
        self.name_is_dirty(cx)
            || !self.drafts.is_empty()
            || self
                .form
                .as_ref()
                .is_some_and(|form| form.has_local_input(cx))
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

    /// Build the form again from what the file says now — the Revert path.
    /// Keeps the same node selected; only the boxes go back.
    pub(in crate::workspace) fn rebuild_form(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drafts.clear();
        self.save_error = None;
        self.name_input.update(cx, |input, cx| {
            input.set_value(self.saved_name.clone(), window, cx)
        });
        cx.notify();
        let Some(node) = self.form.as_ref().map(|form| form.node.clone()) else {
            return;
        };
        let text = self.text.clone();
        let issues = self.current_issues();
        self.form = text.and_then(|text| form::NodeForm::build(&text, &node, &issues, window, cx));
        self.watch_form(window, cx);
        cx.notify();
    }
}
