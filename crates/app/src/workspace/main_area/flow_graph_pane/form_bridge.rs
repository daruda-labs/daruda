//! The bridge between the pane and the node inspector.
//!
//! The form itself is [`super::form`]'s; this is only what the pane does to it —
//! build it, put a refusal on it, and say so when a reload took away something
//! that had been typed.

use daruda_flow::NodeId;
use gpui::{App, Context, Window};

use super::{FlowGraphEvent, FlowGraphView, form, policy};

impl FlowGraphView {
    /// The form for the selected node, when there is one.
    pub(in crate::workspace) fn form(&self) -> Option<&form::NodeForm> {
        self.form.as_ref()
    }

    /// Is the inspector holding something the file has not been told about?
    ///
    /// `Pane::is_dirty` says `false` for a graph on purpose — the pane is a view
    /// of a file, not a buffer over it, so it never joins the close prompt. This
    /// is the narrower question the toolbar asks: running reads the file, so
    /// while these two disagree, ▶ would run something other than what is on
    /// screen.
    pub(in crate::workspace) fn has_unsaved_form(&self, cx: &App) -> bool {
        self.form.as_ref().is_some_and(|form| form.is_dirty(cx))
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
        let Some(node) = self.form.as_ref().map(|form| form.node.clone()) else {
            return;
        };
        let text = self.text.clone();
        let issues = self.current_issues();
        self.form = text.and_then(|text| form::NodeForm::build(&text, &node, &issues, window, cx));
        cx.notify();
    }

    /// Say so when a reload replaced something the person had typed.
    ///
    /// Not "was the form dirty": the pane that pressed Save is dirty too at the
    /// moment its own write comes back, and saying it on every successful save
    /// would be noise. What is actually lost is the difference between what they
    /// had and what the file now says — zero for the pane that wrote it, and the
    /// typing itself for a pane that did not. A node that is gone after the
    /// reload counts too: nothing came back to compare against.
    ///
    /// Emitted rather than written onto the form, because the form is the thing
    /// being replaced. Adding a node rebuilds it for the new one, and a node
    /// renamed out from under the inspector leaves no form at all — either way a
    /// banner would be gone before it was read.
    pub(super) fn say_if_typing_was_dropped(
        &mut self,
        typed: Option<(NodeId, form::NodeFields)>,
        cx: &mut Context<Self>,
    ) {
        let Some((node, fields)) = typed else {
            return;
        };
        let rebuilt = self
            .form
            .as_ref()
            .map(|form| (form.node.clone(), form.fields(cx)));
        let rebuilt = rebuilt.as_ref().map(|(node, fields)| (node, fields));
        if policy::typing_survived(&node, &fields, rebuilt) {
            return;
        }
        cx.emit(FlowGraphEvent::TypingDropped);
    }
}
