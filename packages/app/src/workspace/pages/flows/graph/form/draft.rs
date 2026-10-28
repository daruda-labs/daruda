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
    memo: MatchMemo,
    inputs: Vec<String>,
    choices: Vec<String>,
    agent_open: bool,
    settings_open: bool,
}

impl NodeDraft {
    pub(in crate::workspace) fn source_matches(&self, text: &str) -> bool {
        self.memo.check(&self.source, text, &self.node)
    }

    /// Adopt a reloaded file that left this node and the defaults unchanged,
    /// so later checks take the no-parse path.
    pub(in crate::workspace) fn adopt_if_unchanged(&mut self, text: &str) {
        if self.source != text && self.source_matches(text) {
            self.source = text.to_string();
        }
    }

    /// Carry a draft across an edit this pane made to the file. Typed values
    /// stay; the node's `deps` take the edit's added and removed edges, and
    /// references to deleted nodes go.
    pub(in crate::workspace) fn rebase(
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
        let (old, new) = (file_deps(before, &self.node), file_deps(after, &self.node));
        if old == new && deleted.is_empty() {
            // Nothing typed can refer to what changed, so only the base moves;
            // rebuilding a form here would cost a full set of entities.
            let mut moved = self;
            moved.source = after.to_string();
            return moved;
        }
        let mut rebased = self.clone();
        rebased.source = after.to_string();
        let Some(form) = rebased.restore(issues, window, cx) else {
            return self;
        };
        let typed = super::fields::parse_deps(&form.deps.read(cx).value());
        let deps = rebase_deps(&typed, &old, &new, deleted).join(", ");
        form.deps
            .update(cx, |input, cx| input.set_value(deps, window, cx));
        let rerun = super::fields::parse_deps(&form.body.on_fail.rerun.read(cx).value())
            .into_iter()
            .filter(|id| !is_deleted(deleted, id))
            .collect::<Vec<_>>()
            .join(", ");
        form.body
            .on_fail
            .rerun
            .update(cx, |input, cx| input.set_value(rerun, window, cx));
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
            memo: MatchMemo::default(),
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
        self.match_memo.check(&self.source_text, text, &self.node)
    }

    /// Whether the form was built from exactly `text`.
    pub(in crate::workspace) fn is_built_from(&self, text: &str) -> bool {
        self.source_text == text
    }

    /// Adopt a reloaded file that left this node and the defaults unchanged.
    pub(in crate::workspace) fn rebase_source(&mut self, text: &str) {
        self.source_text = text.to_string();
    }
}

/// The last conflict answer for one pair of texts. Render asks every frame,
/// and a draft in conflict would otherwise parse both files each time.
#[derive(Clone, Default)]
pub(super) struct MatchMemo(std::cell::Cell<Option<(u64, bool)>>);

impl MatchMemo {
    pub(super) fn check(&self, source: &str, text: &str, node: &NodeId) -> bool {
        use std::hash::{Hash as _, Hasher as _};
        if source == text {
            return true;
        }
        let mut hasher = std::hash::DefaultHasher::new();
        (source, text, node.as_str()).hash(&mut hasher);
        let key = hasher.finish();
        if let Some((seen, answer)) = self.0.get()
            && seen == key
        {
            return answer;
        }
        let answer = source_matches(source, text, node);
        self.0.set(Some((key, answer)));
        answer
    }
}

fn file_deps(text: &str, node: &NodeId) -> Vec<String> {
    daruda_flow::parse::parse_flow_file(text)
        .ok()
        .and_then(|file| file.nodes.into_iter().find(|n| &n.id == node))
        .map(|n| n.deps.iter().map(ToString::to_string).collect())
        .unwrap_or_default()
}

fn is_deleted(deleted: &[NodeId], id: &str) -> bool {
    deleted.iter().any(|node| node.as_str() == id)
}

/// Typed dependencies, minus edges the edit removed and nodes it deleted,
/// plus edges it added.
fn rebase_deps(
    typed: &[String],
    before: &[String],
    after: &[String],
    deleted: &[NodeId],
) -> Vec<String> {
    let mut deps: Vec<String> = typed
        .iter()
        .filter(|dep| (!before.contains(dep) || after.contains(dep)) && !is_deleted(deleted, dep))
        .cloned()
        .collect();
    for dep in after {
        if !before.contains(dep) && !deps.contains(dep) {
            deps.push(dep.clone());
        }
    }
    deps
}

pub(super) fn source_matches(before: &str, after: &str, node: &NodeId) -> bool {
    // The common case — nothing reloaded since the form was built — needs no
    // parse; render asks this every frame.
    if before == after {
        return true;
    }
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

    #[test]
    fn rebased_deps_keep_typed_edges_and_take_the_edit() {
        let strings = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            rebase_deps(
                &strings(&["a", "typed"]),
                &strings(&["a"]),
                &strings(&["new"]),
                &[]
            ),
            strings(&["typed", "new"])
        );
        assert_eq!(
            rebase_deps(
                &strings(&["a", "b"]),
                &strings(&["a", "b"]),
                &strings(&["b"]),
                &["a".into()]
            ),
            strings(&["b"])
        );
        assert_eq!(
            rebase_deps(&strings(&["new"]), &[], &strings(&["new"]), &[]),
            strings(&["new"])
        );
    }
}
