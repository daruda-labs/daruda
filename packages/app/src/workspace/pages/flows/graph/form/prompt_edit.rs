//! Optimistic prompt edits resolve the current form instead of retaining widgets.
use super::super::FlowGraphView;
use super::NodeForm;
use daruda_flow::NodeId;
use gpui::{App, Context, Window};
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub(in crate::workspace) enum PromptField {
    Inline,
    File,
}

#[derive(Clone)]
pub(in crate::workspace) struct PromptEdit {
    path: PathBuf,
    node: NodeId,
    source: String,
    field: PromptField,
    expected: String,
}

impl PromptField {
    fn input(self, form: &NodeForm) -> &gpui::Entity<crate::ui::InputState> {
        match self {
            Self::Inline => &form.body.prompt.inline,
            Self::File => &form.body.prompt.file,
        }
    }
}

impl FlowGraphView {
    pub(in crate::workspace) fn begin_prompt_edit(
        &self,
        field: PromptField,
        cx: &App,
    ) -> Option<PromptEdit> {
        let form = self.form.as_ref()?;
        Some(PromptEdit {
            path: self.path.clone(),
            node: form.node.clone(),
            source: form.source_text.clone(),
            field,
            expected: field.input(form).read(cx).value().to_string(),
        })
    }

    /// A changed source or concurrent field edit is refused without losing the
    /// caller's draft. Hidden-node results are kept as plain snapshots.
    pub(in crate::workspace) fn apply_prompt_edit(
        &mut self,
        edit: &PromptEdit,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.path != edit.path
            || self
                .text
                .as_deref()
                .is_none_or(|text| !super::draft::source_matches(&edit.source, text, &edit.node))
        {
            return false;
        }
        if let Some(form) = self.form.as_ref().filter(|form| form.node == edit.node) {
            if !form.source_matches(&edit.source)
                || edit.field.input(form).read(cx).value().as_ref() != edit.expected
            {
                return false;
            }
            edit.field
                .input(form)
                .update(cx, |input, cx| input.set_value(value, window, cx));
        } else {
            let issues = self.current_issues();
            let form = match self.drafts.get(&edit.node).cloned() {
                Some(draft) => draft.restore(&issues, window, cx),
                None => self
                    .text
                    .clone()
                    .and_then(|text| NodeForm::build(&text, &edit.node, &issues, window, cx)),
            };
            let Some(form) = form else {
                return false;
            };
            if !form.source_matches(&edit.source)
                || edit.field.input(&form).read(cx).value().as_ref() != edit.expected
            {
                return false;
            }
            edit.field
                .input(&form)
                .update(cx, |input, cx| input.set_value(value, window, cx));
            self.drafts.insert(edit.node.clone(), form.snapshot(cx));
        }
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {
    // Reload, conflict and hidden-node behavior use the workspace authoring tests.
}
