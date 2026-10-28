//! Plain snapshots retain inactive fields without keeping hidden input entities.
use super::{NodeFields, NodeForm};
use daruda_flow::NodeId;
use gpui::{App, Context, Window};

#[derive(Clone)]
pub(in crate::workspace) struct NodeDraft {
    pub node: NodeId,
    pub fields: NodeFields,
    pub invalid: bool,
    source: String,
    inputs: Vec<String>,
    choices: Vec<String>,
    agent_open: bool,
    settings_open: bool,
}

impl NodeDraft {
    pub(in crate::workspace) fn source_matches(&self, text: &str) -> bool {
        source_matches(&self.source, text, &self.node)
    }

    pub(in crate::workspace) fn after_deletion(
        self,
        deleted: &[NodeId],
        before: &str,
        after: &str,
        issues: &[daruda_flow::error::ValidationIssue],
        window: &mut Window,
        cx: &mut Context<super::super::FlowGraphView>,
    ) -> Self {
        if !self.source_matches(before) {
            return self;
        }
        let mut rebased = self.clone();
        rebased.source = after.to_string();
        let Some(form) = rebased.restore(issues, window, cx) else {
            return self;
        };
        for input in [&form.deps, &form.body.on_fail.rerun] {
            let value = super::fields::parse_deps(&input.read(cx).value())
                .into_iter()
                .filter(|id| !deleted.iter().any(|node| node.as_str() == id))
                .collect::<Vec<_>>()
                .join(", ");
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        form.snapshot(cx)
    }

    pub(in crate::workspace) fn restore(
        self,
        issues: &[daruda_flow::error::ValidationIssue],
        window: &mut Window,
        cx: &mut Context<super::super::FlowGraphView>,
    ) -> Option<NodeForm> {
        let mut form = NodeForm::build(&self.source, &self.node, issues, window, cx)?;
        for (input, value) in form.text_inputs().into_iter().zip(self.inputs) {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        for (select, value) in form.draft_selects().into_iter().zip(self.choices) {
            select.update(cx, |select, cx| {
                select.set_selected_value(&value.into(), window, cx)
            });
        }
        form.agent_open = self.agent_open;
        form.settings_open = self.settings_open;
        Some(form)
    }
}

impl NodeForm {
    pub(in crate::workspace) fn snapshot(&self, cx: &App) -> NodeDraft {
        NodeDraft {
            node: self.node.clone(),
            fields: self.fields(cx),
            invalid: self.refusal(cx).is_some(),
            source: self.source_text.clone(),
            inputs: self.text_snapshot(cx),
            choices: self
                .draft_selects()
                .iter()
                .map(|select| {
                    select
                        .read(cx)
                        .selected_value()
                        .map(ToString::to_string)
                        .unwrap_or_default()
                })
                .collect(),
            agent_open: self.agent_open,
            settings_open: self.settings_open,
        }
    }

    fn draft_selects(&self) -> [&gpui::Entity<crate::ui::select::SelectState>; 5] {
        [
            &self.body.kind,
            &self.body.prompt.choice,
            &self.agent.permission,
            &self.body.on_fail.policy,
            &self.body.on_fail.hint.choice,
        ]
    }

    pub(in crate::workspace) fn source_matches(&self, text: &str) -> bool {
        source_matches(&self.source_text, text, &self.node)
    }
}

pub(super) fn source_matches(before: &str, after: &str, node: &NodeId) -> bool {
    let parse = daruda_flow::parse::parse_flow_file;
    match (parse(before), parse(after)) {
        (Ok(before), Ok(after)) => {
            before.defaults == after.defaults
                && before.nodes.iter().find(|n| &n.id == node)
                    == after.nodes.iter().find(|n| &n.id == node)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unrelated_comments_do_not_conflict_but_node_changes_do() {
        let text = "version: 1\nnodes:\n  - id: build\n    kind: command\n    run: echo ok\n";
        assert!(source_matches(
            text,
            &format!("{text}# comment\n"),
            &"build".into()
        ));
        assert!(!source_matches(
            text,
            &text.replace("echo ok", "echo changed"),
            &"build".into()
        ));
    }
}
