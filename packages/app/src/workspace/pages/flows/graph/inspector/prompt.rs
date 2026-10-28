//! Optimistic edits compare the file revision and raw field without widgets.
use super::{baseline::Baseline, draft::NodeDraft, model::InspectorModel};
use daruda_flow::NodeId;

#[derive(Clone, Copy)]
pub(in crate::workspace) enum PromptField {
    Inline,
    File,
}

#[derive(Clone)]
pub(in crate::workspace) struct PromptEdit {
    node: NodeId,
    base: Baseline,
    field: PromptField,
    expected: String,
}

impl PromptField {
    fn value(self, draft: &NodeDraft) -> &str {
        match self {
            Self::Inline => &draft.raw().prompt_inline,
            Self::File => &draft.raw().prompt_file,
        }
    }
}

impl InspectorModel {
    pub(in crate::workspace) fn begin_prompt(&self, field: PromptField) -> Option<PromptEdit> {
        let draft = self.draft()?;
        Some(PromptEdit {
            node: draft.node().clone(),
            base: draft.baseline().clone(),
            field,
            expected: field.value(draft).into(),
        })
    }
    pub(in crate::workspace) fn apply_prompt(&mut self, edit: &PromptEdit, value: String) -> bool {
        let Some(base) = &self.baseline else {
            return false;
        };
        if !edit.base.matches(base, &edit.node) || base.node(&edit.node).is_none() {
            return false;
        }
        let Some(mut draft) = self
            .drafts
            .get(&edit.node)
            .cloned()
            .or_else(|| NodeDraft::new(base.clone(), edit.node.clone()))
        else {
            return false;
        };
        if !draft.matches(&edit.base) || edit.field.value(&draft) != edit.expected {
            return false;
        }
        let mut raw = draft.raw().clone();
        match edit.field {
            PromptField::Inline => raw.prompt_inline = value,
            PromptField::File => raw.prompt_file = value,
        }
        draft.edit(raw, draft.ui().clone());
        self.drafts.insert(edit.node.clone(), draft);
        self.debug_assert_invariants();
        true
    }
}

#[cfg(test)]
mod tests {}
