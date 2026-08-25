//! Reading a flow file and building the canvas that draws it.
//!
//! Every open places the nodes again: the file declares dependencies, not
//! positions, and coordinates are never persisted. Which plugins the canvas
//! carries is decided here too — that list is what makes it behave like a flow
//! graph rather than a general node editor.

use std::collections::HashMap;
use std::path::Path;

use daruda_flow::NodeId;
use gpui::{AppContext as _, Context, Window};

use super::click::NodeClickPlugin;
use super::disconnect::EdgeDeletePlugin;
use super::frame::FrameGraphPlugin;
use super::model::{FlowGraphModel, NodeRunState};
use super::node_ids::NodeIds;
use super::overlay::RerunOverlay;
use super::pins::PinSet;
use super::port_drag::FlowEdgeValidator;
use super::renderer::{FlowNodeRenderer, NODE_TYPE, card_for, flow_theme};
use super::{FlowGraphError, FlowGraphState, FlowGraphView, renderer};
use crate::ui::flow_canvas::{
    BackgroundPlugin, CanvasNodeId, FlowCanvas, Graph, GraphPlugin, PortInteractionPlugin,
    SelectionPlugin, ViewportPlugin,
    layout::{LayeredDagLayout, LayoutOptions, LayoutOutput, LayoutStrategy, PositionHint},
};
use crate::ui::theme::palette;

/// Read the file, keeping the text: the graph is derived from it and
/// [`FlowGraphView::reload`] compares against it.
pub(super) fn read_flow(path: &Path) -> Result<String, FlowGraphError> {
    std::fs::read_to_string(path).map_err(|e| FlowGraphError::Read {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

/// Turn a model into a canvas: the graph, the plugins that make it behave the
/// way this pane needs, and the id map between the two.
///
/// The pins come in with the model because a card is `(node, state, pinned)`
/// and this is the one place the cards are first written. Stamping them on
/// afterwards would flash a graph with the pins missing.
pub(super) fn build_graph_state(
    model: FlowGraphModel,
    pins: &PinSet,
    window: &mut Window,
    cx: &mut Context<FlowGraphView>,
) -> FlowGraphState {
    let (graph, ids) = build_canvas_graph(&model, pins);
    // Resolved here, where the colours are readable: neither the canvas's own
    // theme nor a node renderer can reach `cx` once the canvas is built.
    let tokens = crate::ui::theme::PaneSurfaceTokens::flow_graph(cx);
    let rerun = RerunOverlay::new(ids.clone(), model.rerun.clone());
    let canvas = cx.new(|c| {
        // Deliberately not `default_plugins()`: no node drag (auto-placed
        // nodes carry no position to move) and no history (the YAML file
        // is the single undo stack). Framing is daruda's own (`frame.rs`):
        // a graph laid out one column-pitch apart outgrows a pane, and how
        // far out it is worth zooming depends on what our cards say when
        // they get there.
        FlowCanvas::builder(graph, c, window)
            .theme(flow_theme(&tokens))
            .node_renderer(
                NODE_TYPE,
                FlowNodeRenderer {
                    palette: renderer::CardPalette::of(&tokens),
                },
            )
            .plugin(BackgroundPlugin::new())
            .plugin(ViewportPlugin::new())
            .plugin(GraphPlugin::new())
            .plugin(SelectionPlugin::new())
            .plugin(NodeClickPlugin::new())
            // Dragging port to port is how a dependency is drawn. The
            // validator is ours, so a refusal shows while the wire is still
            // being dragged rather than after it is written. Dangling links are
            // off: a release that landed on no port ends there, and a node the
            // file never declared is not something to offer.
            .plugin(
                PortInteractionPlugin::new()
                    .validator(FlowEdgeValidator)
                    .dangling_links(false),
            )
            // Delete on a selected line takes it away again.
            .plugin(EdgeDeletePlugin)
            .plugin(FrameGraphPlugin::new())
            .plugin(rerun)
            .build()
    });
    FlowGraphState::Graph { canvas, model, ids }
}

/// Build the canvas graph and lay it out. Coordinates are never persisted —
/// the flow file declares dependencies, not positions — so every open
/// places the nodes again.
fn build_canvas_graph(model: &FlowGraphModel, pins: &PinSet) -> (Graph, NodeIds) {
    let mut ids: HashMap<NodeId, CanvasNodeId> = HashMap::new();
    let mut inputs = HashMap::new();
    let mut outputs = HashMap::new();

    let mut graph = Graph::build(|g| {
        for node in &model.nodes {
            let card = card_for(node, NodeRunState::default(), pins.contains(&node.id));
            let (nid, ins, outs) = g
                .create_node(NODE_TYPE)
                .size(palette::FLOW_GRAPH_NODE_W, palette::FLOW_GRAPH_NODE_H)
                .input()
                .output()
                .data(serde_json::to_value(&card).unwrap_or_default())
                .build_with_ports();
            ids.insert(node.id.clone(), nid);
            inputs.insert(node.id.clone(), ins);
            outputs.insert(node.id.clone(), outs);
        }
        for edge in &model.deps {
            let (Some(from), Some(to)) = (outputs.get(&edge.from), inputs.get(&edge.to)) else {
                continue;
            };
            let (Some(from), Some(to)) = (from.first(), to.first()) else {
                continue;
            };
            g.create_edge().source(*from).target(*to).build();
        }
    });

    if let Ok(LayoutOutput::Delta(delta)) =
        LayeredDagLayout.compute(&graph, &LayoutOptions::default(), None)
    {
        // `NodePositionDelta`'s own fields are crate-private upstream;
        // `PositionHint` is the public way to read the result.
        let placed: Vec<_> = PositionHint::from_delta_to(&delta)
            .positions()
            .iter()
            .map(|(id, p)| (*id, *p))
            .collect();
        for (id, at) in placed {
            if let Some(node) = graph.get_node_mut(&id) {
                node.set_position_with_point(at);
            }
        }
    }
    (graph, NodeIds::new(ids))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_of(yaml: &str) -> FlowGraphModel {
        let loaded = daruda_flow::load(yaml, None).expect("fixture should load");
        FlowGraphModel::from_flow(loaded.flow())
    }

    const ONE_NODE: &str = concat!(
        "version: 1\n",
        "defaults:\n",
        "  agent:\n",
        "    id: claude\n",
        "    mode: bypassPermissions\n",
        "nodes:\n",
        "  - id: hello\n",
        "    kind: agent\n",
        "    output: hello.md\n",
        "    prompt: hi\n",
    );

    /// A single node still has to land somewhere the viewport can see, and
    /// still has to carry the card the renderer reads back.
    #[test]
    fn a_lone_node_is_placed_and_stamped() {
        let (graph, ids) = build_canvas_graph(&model_of(ONE_NODE), &PinSet::default());
        assert!(
            ids.canvas(&NodeId::from("hello")).is_some(),
            "the flow id maps to a canvas node"
        );
        let node = graph.nodes().values().next().expect("one node");
        let (x, y) = node.position();
        assert!(
            f32::from(x).is_finite() && f32::from(y).is_finite(),
            "placed at ({x:?}, {y:?})"
        );
        let card: renderer::CardData =
            serde_json::from_value(node.data_ref().clone()).expect("the node carries a card");
        assert_eq!(card.id, "hello");
    }
}
