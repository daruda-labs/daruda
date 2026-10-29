//! A selected node remains inspectable before it has any attempt evidence.

use daruda_flow::{NodeId, history::RunHistory};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum ExecutionSelection {
    Node(NodeId),
    Attempt(NodeId, u32),
}

impl ExecutionSelection {
    pub fn for_node(history: &RunHistory, node: NodeId) -> Self {
        match history.attempts(&node).last() {
            Some((_, evidence)) => Self::Attempt(node, *evidence),
            None => Self::Node(node),
        }
    }

    pub fn node(&self) -> &NodeId {
        match self {
            Self::Node(node) | Self::Attempt(node, _) => node,
        }
    }

    pub fn attempt(&self) -> Option<(&NodeId, &u32)> {
        match self {
            Self::Attempt(node, evidence) => Some((node, evidence)),
            Self::Node(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_selection_does_not_invent_evidence() {
        let selected = ExecutionSelection::Node("pending".into());
        assert_eq!(selected.node().as_str(), "pending");
        assert!(selected.attempt().is_none());
    }
}
