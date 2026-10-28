//! Prompt editor and native picker results enter through the model.
use super::super::{FlowGraphView, inspector};
use gpui::{App, Context, Window};
use std::path::PathBuf;

pub(in crate::workspace) use inspector::PromptField;

#[derive(Clone)]
pub(in crate::workspace) struct PromptEdit {
    path: PathBuf,
    edit: inspector::PromptEdit,
}

impl FlowGraphView {
    pub(in crate::workspace) fn begin_prompt_edit(
        &self,
        field: PromptField,
        _cx: &App,
    ) -> Option<PromptEdit> {
        Some(PromptEdit {
            path: self.path.clone(),
            edit: self.inspector.begin_prompt(field)?,
        })
    }

    pub(in crate::workspace) fn apply_prompt_edit(
        &mut self,
        edit: &PromptEdit,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.sync_form(cx);
        if self.path != edit.path || !self.inspector.apply_prompt(&edit.edit, value) {
            return false;
        }
        self.project_form(window, cx);
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {}
