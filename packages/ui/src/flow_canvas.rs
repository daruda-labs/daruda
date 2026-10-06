//! Wrapper home for the vendored `ferrum_flow` node-graph canvas.
//!
//! The same rule `gpui_component` lives under: application code never
//! names `ferrum_flow` directly, it goes through `crate::ui::flow_canvas`.
//! Enforced by `scripts/lint-direct-ferrum-flow.sh`.
//!
//! Only what the flow editor actually uses is re-exported. Opening a name
//! before there is a call site would make the vendor surface look wider
//! than daruda's dependence on it really is, and the point of this module
//! is that the surface stays small enough to replace.

/// The plugin set daruda composes by hand instead of taking
/// `default_plugins()`. Two omissions are deliberate:
/// `NestedNodeDragPlugin` (nodes are auto-placed, so dragging one carries
/// no information) and `HistoryPlugin` (the YAML buffer is the single undo
/// stack — a second one would make ⌘Z ambiguous).
///
/// Framing the graph into the pane is daruda's own plugin rather than the
/// vendored `FitAllGraphPlugin`: upstream's fit magnifies up to 3× and stops
/// zooming out at 0.7, and both of those are decisions about what a card
/// still says at that size — which daruda's cards answer differently.
pub use ferrum_flow::{BackgroundPlugin, GraphPlugin, SelectionPlugin, ViewportPlugin};

/// The eighth plugin, and the only one that writes to the flow file: dragging
/// between two ports is how a dependency is declared. It takes a validator, so
/// daruda's own rules are enforced while the wire is still being dragged —
/// which is the whole reason no vendor patch was needed for it.
///
/// `DefaultEdgeValidator` is re-exported alongside because daruda's validator
/// delegates to it rather than restating "not to itself, one output and one
/// input" — rules that are the vendor's to keep.
pub use ferrum_flow::{
    DefaultEdgeValidator, EdgeValidationError, EdgeValidator, PortInteractionPlugin,
};

/// An edge as the canvas holds it, for a host reading its graph back. `Edge`'s
/// ports are what say which nodes it joins; `EdgeId` is what removes it.
/// `PortKind` tells the two apart, which is what a test drawing an edge by
/// hand needs.
pub use ferrum_flow::{Edge, EdgeId, PortId, PortKind};

/// What a node removal does with the node's children. daruda's graph is flat —
/// a flow file declares no nesting — so the choice never matters in itself;
/// `Cascade` is passed because a node being removed is one the file does not
/// name, and promoting anything under it would keep exactly that.
pub use ferrum_flow::ParentDeletePolicy;
pub use ferrum_flow::{
    FlowCanvas, FlowTheme, Graph, Node, NodeRenderer, Plugin, Port, RenderContext, RenderLayer,
    Viewport,
};

/// Renamed on the way through: a flow node's id is `daruda_flow::NodeId`, and
/// an unqualified "node id" in a flow editor means that one. This is the
/// canvas's own handle for a drawn node — a vendored implementation detail
/// this module exists to hold at arm's length — so it is the one that takes
/// the qualifier. `scripts/lint-direct-ferrum-flow.sh` is what makes the
/// rename total: there is no other way in.
pub use ferrum_flow::NodeId as CanvasNodeId;

/// The canvas exposes its graph immutably and takes edits as commands, so a
/// host that changes anything — re-stamping a card from a run, framing the
/// graph into the pane — implements this rather than reaching for the graph.
/// `PluginContext` has no public viewport setter; `CommandContext` does.
pub use ferrum_flow::{Command, CommandContext};

/// What a daruda-side plugin needs to answer an event: the two it handles are
/// "the pane has been measured" and ⌘0.
pub use ferrum_flow::{
    EventResult, FlowEvent, InputEvent, PluginContext, primary_platform_modifier,
};

/// A drag that outlives the press that began it — the canvas routes moves and
/// the release to it until it ends. Here because panning the view is one, and
/// the vendor's own pan is not reachable to add a third way into.
pub use ferrum_flow::{Interaction, InteractionResult};

pub mod layout {
    //! Automatic placement. daruda never persists node coordinates — the
    //! flow file declares dependencies, not positions — so every graph is
    //! laid out on load.
    pub use ferrum_flow::layout::{
        LayeredDagLayout, LayoutOptions, LayoutOutput, LayoutStrategy, PositionHint,
    };
}

#[cfg(test)]
mod tests {
    use super::layout::{
        LayeredDagLayout, LayoutOptions, LayoutOutput, LayoutStrategy, PositionHint,
    };
    use super::*;

    /// Node box the layout assertions below are calibrated to.
    ///
    /// `layered_dag` spaces columns by `max_node_width + layer_spacing`
    /// (default 120), so the pitch moves with whatever size the caller
    /// declares. Pinning it here is what makes the expected coordinates
    /// mean anything.
    const NODE_W: f32 = 150.0;
    const NODE_H: f32 = 48.0;
    const EXPECTED_PITCH: f32 = NODE_W + 120.0;

    /// The design doc's representative flow: a five-node chain.
    fn chain() -> Graph {
        Graph::build(|g| {
            let mut ins = Vec::new();
            let mut outs = Vec::new();
            for id in ["design", "implement", "test", "review", "review-gate"] {
                // Scattered on purpose — the point is that layout fixes it.
                let (_, i, o) = g
                    .create_node("flow")
                    .position(40.0, 60.0)
                    .size(NODE_W, NODE_H)
                    .input()
                    .output()
                    .data(serde_json::json!({ "id": id }))
                    .build_with_ports();
                ins.push(i);
                outs.push(o);
            }
            for k in 0..outs.len() - 1 {
                g.create_edge()
                    .source(outs[k][0])
                    .target(ins[k + 1][0])
                    .build();
            }
        })
    }

    /// The vendored crate does real work through this module: a chain laid
    /// out left to right, one column per node, all on one row.
    #[test]
    fn layered_dag_places_a_chain_one_column_apart() {
        let graph = chain();
        let out = LayeredDagLayout
            .compute(&graph, &LayoutOptions::default(), None)
            .expect("layout should succeed on an acyclic chain");

        let LayoutOutput::Delta(delta) = out else {
            panic!("a scattered chain has moves to make");
        };
        // `NodePositionDelta`'s fields are crate-private upstream;
        // `PositionHint` is the public way to read the result.
        let hint = PositionHint::from_delta_to(&delta);
        let mut xs: Vec<f32> = hint.positions().values().map(|p| f32::from(p.x)).collect();
        let ys: Vec<f32> = hint.positions().values().map(|p| f32::from(p.y)).collect();
        xs.sort_by(f32::total_cmp);

        assert_eq!(xs.len(), 5, "every node should be placed");
        for (k, x) in xs.iter().enumerate() {
            assert_eq!(
                *x,
                k as f32 * EXPECTED_PITCH,
                "column {k} should sit one pitch from the last"
            );
        }
        assert!(
            ys.windows(2).all(|w| w[0] == w[1]),
            "a chain is one layer deep, so every node shares a row: {ys:?}"
        );
    }
}
