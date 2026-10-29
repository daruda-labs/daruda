//! Link candidates use the cached file plus every pending draft.
use super::model::InspectorModel;
use daruda_flow::{
    NodeId,
    parse::{FlowFile, NodeKindFile},
};
use std::collections::BTreeSet;

impl InspectorModel {
    pub(in crate::workspace) fn link_candidates(&self) -> Option<(Vec<String>, Vec<String>)> {
        let draft = self.draft()?;
        let mut file = self.baseline.as_ref()?.file()?.clone();
        for (id, mut fields) in self.pending_fields() {
            fields.id = id.to_string();
            super::apply::node_fields(&mut file, &id, &fields);
        }
        Some(candidates(&file, draft.node(), &draft.fields().deps))
    }
}

pub(super) fn candidates(
    file: &FlowFile,
    node: &NodeId,
    deps: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut ancestors: BTreeSet<String> = deps.iter().cloned().collect();
    loop {
        let before = ancestors.len();
        for entry in &file.nodes {
            if ancestors.contains(entry.id.as_str()) {
                ancestors.extend(entry.deps.iter().map(ToString::to_string));
            }
        }
        if ancestors.len() == before {
            break;
        }
    }
    let mut descendants = BTreeSet::from([node.to_string()]);
    loop {
        let before = descendants.len();
        for entry in &file.nodes {
            if entry
                .deps
                .iter()
                .any(|dep| descendants.contains(dep.as_str()))
            {
                descendants.insert(entry.id.to_string());
            }
        }
        if descendants.len() == before {
            break;
        }
    }
    let available = file
        .nodes
        .iter()
        .filter(|n| {
            !descendants.contains(n.id.as_str()) && !deps.iter().any(|dep| dep == n.id.as_str())
        })
        .map(|n| n.id.to_string())
        .collect();
    let outputs = file
        .nodes
        .iter()
        .filter(|n| {
            n.id != *node
                && ancestors.contains(n.id.as_str())
                && matches!(n.kind, NodeKindFile::Agent { .. })
        })
        .map(|n| n.id.to_string())
        .collect();
    (available, outputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dependencies_exclude_cycles_and_results_require_agent_ancestors() {
        let file = daruda_flow::parse::parse_flow_file("version: 1\nnodes:\n  - id: a\n    kind: agent\n    prompt: hello\n    output: a.md\n  - id: b\n    deps: [a]\n    kind: command\n    run: echo ok\n  - id: c\n    deps: [b]\n    kind: agent\n    prompt: hello\n    output: c.md\n").unwrap();
        let (deps, outputs) = candidates(&file, &NodeId::from("b"), &["a".into()]);
        assert!(deps.is_empty());
        assert_eq!(outputs, ["a"]);
        let (_, outputs) = candidates(&file, &NodeId::from("c"), &["b".into()]);
        assert_eq!(outputs, ["a"]);
    }
}
