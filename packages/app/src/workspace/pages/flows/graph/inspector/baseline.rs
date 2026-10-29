//! One immutable file revision, parsed once for all inspector operations.
use daruda_flow::error::ValidationIssue;
use daruda_flow::{
    NodeId,
    parse::{FlowFile, NodeFile},
};
use std::sync::Arc;

/// The text, its parse, and what the engine refuses about it — one revision.
#[derive(Clone)]
pub(super) struct Baseline {
    text: Arc<str>,
    file: Option<Arc<FlowFile>>,
    issues: Arc<[ValidationIssue]>,
}

impl Baseline {
    pub(super) fn new(text: Arc<str>, issues: Vec<ValidationIssue>) -> Self {
        let file = daruda_flow::parse::parse_flow_file(&text)
            .ok()
            .map(Arc::new);
        Self {
            text,
            file,
            issues: issues.into(),
        }
    }

    /// A revision parsed for comparison only; it carries no issues to show.
    pub(super) fn bare(text: Arc<str>) -> Self {
        Self::new(text, Vec::new())
    }

    pub(super) fn issues(&self) -> &[ValidationIssue] {
        &self.issues
    }

    pub(super) fn text(&self) -> &Arc<str> {
        &self.text
    }
    pub(super) fn file(&self) -> Option<&FlowFile> {
        self.file.as_deref()
    }
    pub(super) fn node(&self, node: &NodeId) -> Option<&NodeFile> {
        self.file()?.nodes.iter().find(|entry| &entry.id == node)
    }
    pub(super) fn matches(&self, other: &Self, node: &NodeId) -> bool {
        if Arc::ptr_eq(&self.text, &other.text) || self.text == other.text {
            return true;
        }
        match (self.file(), other.file()) {
            (Some(a), Some(b)) => a.defaults == b.defaults && self.node(node) == other.node(node),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOW: &str = "version: 1\nnodes:\n  - id: a\n    kind: command\n    run: echo a\n  - id: b\n    kind: command\n    run: echo b\n";

    #[test]
    fn a_change_to_another_node_still_matches() {
        let before = Baseline::bare(FLOW.into());
        let after = Baseline::bare(FLOW.replace("echo b", "echo B").into());
        assert!(before.matches(&after, &"a".into()));
        assert!(!before.matches(&after, &"b".into()));
    }
}
