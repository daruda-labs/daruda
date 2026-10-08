//! Where a flow node's id and the canvas node drawing it meet.
//!
//! Both directions are asked for, and by different callers: a card is stamped
//! by flow id, and a selection — or an edge the person just drew — comes back
//! from the canvas needing to be named in the file's vocabulary. Kept as one
//! type because the reverse direction was a linear scan over the forward map
//! at its first call site, and a second caller reading on every canvas notify
//! would make that scan the cost of panning.
//!
//! The reverse map is derived at construction and never maintained beside the
//! forward one: two maps that can be updated separately are two maps that can
//! disagree, and nothing here would say which was right.

use std::collections::HashMap;

use daruda_flow::NodeId;

use crate::ui::flow_canvas::CanvasNodeId;

#[derive(Clone)]
pub(super) struct NodeIds {
    canvas: HashMap<NodeId, CanvasNodeId>,
    flow: HashMap<CanvasNodeId, NodeId>,
}

impl NodeIds {
    /// Build from the forward map the graph build produces. The canvas ids are
    /// fresh per build, so a duplicate cannot arrive; a flow file with two
    /// nodes of one id does not load.
    pub(super) fn new(canvas: HashMap<NodeId, CanvasNodeId>) -> Self {
        let flow = canvas
            .iter()
            .map(|(flow_id, canvas_id)| (*canvas_id, flow_id.clone()))
            .collect();
        Self { canvas, flow }
    }

    /// The canvas node drawing `flow`, if the graph still has it.
    pub(super) fn canvas(&self, flow: &NodeId) -> Option<CanvasNodeId> {
        self.canvas.get(flow).copied()
    }

    /// What the file calls `canvas`. `None` is a canvas node this side cannot
    /// name — nothing to do about it, and not an error.
    pub(super) fn flow(&self, canvas: CanvasNodeId) -> Option<&NodeId> {
        self.flow.get(&canvas)
    }

    /// Every pair, for a caller that walks all the cards. Test-only: the app
    /// itself always has one id in hand and wants the other.
    #[cfg(test)]
    pub(super) fn iter(&self) -> impl Iterator<Item = (&NodeId, CanvasNodeId)> {
        self.canvas.iter().map(|(flow, canvas)| (flow, *canvas))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (NodeIds, CanvasNodeId, CanvasNodeId) {
        let mut graph = crate::ui::flow_canvas::Graph::new();
        let design = graph.create_node("default").build();
        let build = graph.create_node("default").build();
        let ids = NodeIds::new(HashMap::from([
            (NodeId::from("design"), design),
            (NodeId::from("build"), build),
        ]));
        (ids, design, build)
    }

    #[test]
    fn each_direction_answers_the_other() {
        let (ids, design, build) = ids();
        assert_eq!(ids.canvas(&NodeId::from("design")), Some(design));
        assert_eq!(ids.canvas(&NodeId::from("build")), Some(build));
        assert_eq!(ids.flow(design), Some(&NodeId::from("design")));
        assert_eq!(ids.flow(build), Some(&NodeId::from("build")));
    }

    /// The reverse map is derived, so it cannot name a node the forward one
    /// does not — which is the disagreement two hand-kept maps would allow.
    #[test]
    fn the_two_directions_hold_the_same_pairs() {
        let (ids, ..) = ids();
        assert_eq!(ids.flow.len(), ids.canvas.len());
        for (flow_id, canvas_id) in ids.iter() {
            assert_eq!(ids.flow(canvas_id), Some(flow_id), "{flow_id} round-trips");
        }
    }

    #[test]
    fn a_node_neither_side_has_is_nobody() {
        let (ids, ..) = ids();
        assert_eq!(ids.canvas(&NodeId::from("drawing")), None);
        let stranger = crate::ui::flow_canvas::Graph::new()
            .create_node("default")
            .build();
        assert_eq!(ids.flow(stranger), None, "a node from another graph");
    }
}
