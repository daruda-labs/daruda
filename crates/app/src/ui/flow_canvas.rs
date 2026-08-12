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
pub use ferrum_flow::{BackgroundPlugin, GraphPlugin, SelectionPlugin, ViewportPlugin};
pub use ferrum_flow::{
    FlowCanvas, FlowTheme, Graph, Node, NodeId, NodeRenderer, Plugin, Port, RenderContext,
    RenderLayer,
};

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
