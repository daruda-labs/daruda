//! Dragging between two ports, and what the pane does about it.
//!
//! The vendored `PortInteractionPlugin` draws the wire and puts an edge in the
//! canvas graph. It tells nobody. So this side reads the graph back on every
//! canvas notify and holds one invariant:
//!
//! > **what is drawn is what the file declares** — its nodes, and the `deps`
//! > between them.
//!
//! A difference is answered by taking the edge off the canvas — restoring the
//! invariant at once — and asking the workspace to write the file. On a write
//! the reload rebuilds the graph and the line is there again; on a refusal the
//! canvas is already clean and the reason is a toast. Which means **no refusal
//! has to be understood here**: stale file, unwritable file, a cycle the engine
//! caught — all of them leave the same clean picture.
//!
//! Reading the graph rather than taking an event is also what makes this
//! self-healing: a disagreement cannot survive a notify, so there is no state
//! to get stuck. An event, missed once, would leave the two out of step for
//! good.
//!
//! Nodes are in the invariant because the vendored plugin adds one uninvited:
//! a wire dropped on empty space leaves a dangling endpoint, and clicking it
//! builds a blank node plus an edge to it (`finish_pending_link`). Nothing in
//! the file names that node, so it comes off — which is also the decision
//! "a connection joins two cards that already exist", enforced rather than
//! merely intended.

use gpui::Context;

use crate::ui::flow_canvas::{
    CanvasNodeId, Command, CommandContext, DefaultEdgeValidator, EdgeId, EdgeValidationError,
    EdgeValidator, ParentDeletePolicy, PluginContext, Port,
};

use super::model::GraphEdge;
use super::{FlowGraphEvent, FlowGraphState, FlowGraphView, connect};

/// daruda's rules for a connection, on top of the vendor's.
///
/// Delegates first: "not a port to itself", "one output and one input" and the
/// scope rule are the vendor's to keep, and restating them here would be a
/// second copy to drift.
///
/// What it adds is the duplicate. What it deliberately does **not** add is the
/// cycle: refusing one here would show a red wire and no reason, because the
/// message a refusal carries (`FlowEvent::Message`) has no consumer in the
/// vendor or in daruda. Letting it through means the engine refuses it in
/// `edit_flow` instead and a toast says which nodes the cycle runs through.
pub(super) struct FlowEdgeValidator;

impl EdgeValidator for FlowEdgeValidator {
    fn validate(
        &self,
        out_of: &Port,
        into: &Port,
        ctx: &PluginContext,
    ) -> Result<(), EdgeValidationError> {
        DefaultEdgeValidator.validate(out_of, into, ctx)?;
        if links(ctx, out_of.node_id(), into.node_id()) {
            // Never rendered — the wire going red is the whole feedback, and
            // there is nowhere a `FlowEvent::Message` is read. Worded for a
            // developer reading a log, not for `surface::strings`.
            return Err(EdgeValidationError::already_connected(
                "these cards are already connected".into(),
            ));
        }
        Ok(())
    }
}

/// Whether the canvas already runs `into` after `out_of`. Asked of the graph
/// rather than of the model: by the invariant above they say the same thing,
/// and the graph is the one already in hand.
fn links(ctx: &PluginContext, out_of: CanvasNodeId, into: CanvasNodeId) -> bool {
    ctx.graph.edges().values().any(|edge| {
        let (Some(source), Some(target)) = (
            ctx.graph.get_port(&edge.source_port),
            ctx.graph.get_port(&edge.target_port),
        ) else {
            return false;
        };
        source.node_id() == out_of && target.node_id() == into
    })
}

/// Take an edge off the canvas. A command because that is the only public way
/// to write to the graph.
struct DropEdge {
    edge: EdgeId,
}

/// Take nodes the file does not name off the canvas.
///
/// Their ports and every edge touching them go with them — the vendored
/// `Graph::remove_node` cascades — so a blank node and the wire drawn to it
/// are one removal, not two.
struct DropNodes {
    nodes: Vec<CanvasNodeId>,
}

impl Command for DropNodes {
    fn name(&self) -> &'static str {
        "daruda_drop_stray_flow_nodes"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        for node in &self.nodes {
            // `Cascade`: a blank node has no children, and promoting them
            // would be keeping something the file still does not name.
            let _ = ctx.remove_node(node, ParentDeletePolicy::Cascade);
        }
    }

    fn undo(&mut self, _ctx: &mut CommandContext) {}
}

impl Command for DropEdge {
    fn name(&self) -> &'static str {
        "daruda_drop_flow_edge"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        ctx.remove_edge(&self.edge);
    }

    fn undo(&mut self, _ctx: &mut CommandContext) {}
}

impl FlowGraphView {
    /// Hold the invariant, and turn a line the person drew into a file change.
    ///
    /// Runs on every canvas notify — pan and zoom included — so it does no
    /// work when the picture and the file agree, which is almost always.
    pub(super) fn reconcile_edges(&mut self, cx: &mut Context<Self>) {
        let FlowGraphState::Graph { canvas, model, ids } = &self.state else {
            return;
        };
        // Both halves of the invariant, read in one pass. An edge is named in
        // the file's vocabulary before anything is compared; a node the file
        // cannot name is a stray, and so is the edge drawn to it — which is
        // why the two are independent and can be acted on together.
        let (strays, drawn): (Vec<CanvasNodeId>, Vec<(EdgeId, GraphEdge)>) = {
            let graph = canvas.read(cx).graph();
            let strays = graph
                .nodes()
                .keys()
                .filter(|node| ids.flow(**node).is_none())
                .copied()
                .collect();
            let drawn = graph
                .edges()
                .values()
                .filter_map(|edge| {
                    let out_of = ids.flow(graph.get_port(&edge.source_port)?.node_id())?;
                    let into = ids.flow(graph.get_port(&edge.target_port)?.node_id())?;
                    Some((edge.id, connect::dep_from_edge(out_of, into)))
                })
                .collect();
            (strays, drawn)
        };
        let unrecorded = connect::unrecorded(&drawn, &model.deps);
        if strays.is_empty() && unrecorded.is_none() {
            return;
        }
        let canvas = canvas.clone();
        canvas.update(cx, |canvas, cx| {
            if !strays.is_empty() {
                canvas.dispatch_command(DropNodes { nodes: strays }, cx);
            }
            if let Some((edge, _)) = &unrecorded {
                canvas.dispatch_command(DropEdge { edge: *edge }, cx);
            }
        });
        if let Some((_, dep)) = unrecorded {
            cx.emit(FlowGraphEvent::Connect {
                out_of: dep.from,
                into: dep.to,
            });
        }
    }
}

#[cfg(test)]
impl FlowGraphView {
    /// Put an edge on the canvas the way a completed port drag would, so the
    /// reconcile can be exercised without a mouse.
    ///
    /// Stops short of the vendor's drag: what is being tested is what this side
    /// does with an edge that has appeared, which is the half daruda owns.
    pub(in crate::workspace) fn draw_edge_for_test(
        &mut self,
        out_of: &daruda_flow::NodeId,
        into: &daruda_flow::NodeId,
        cx: &mut Context<Self>,
    ) {
        use crate::ui::flow_canvas::PortKind;

        let FlowGraphState::Graph { canvas, ids, .. } = &self.state else {
            return;
        };
        let (Some(out_of), Some(into)) = (ids.canvas(out_of), ids.canvas(into)) else {
            return;
        };
        let canvas = canvas.clone();
        let port =
            |node, kind| {
                canvas.read(cx).graph().ports().values().find_map(|port| {
                    (port.node_id() == node && port.kind() == kind).then(|| port.id())
                })
            };
        let (Some(source), Some(target)) =
            (port(out_of, PortKind::Output), port(into, PortKind::Input))
        else {
            return;
        };
        canvas.update(cx, |canvas, cx| {
            canvas.dispatch_command(AddEdge { source, target }, cx);
        });
    }
}

#[cfg(test)]
struct AddEdge {
    source: crate::ui::flow_canvas::PortId,
    target: crate::ui::flow_canvas::PortId,
}

#[cfg(test)]
impl Command for AddEdge {
    fn name(&self) -> &'static str {
        "daruda_test_add_flow_edge"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        let edge = ctx.new_edge().source(self.source).target(self.target);
        ctx.add_edge(edge);
    }

    fn undo(&mut self, _ctx: &mut CommandContext) {}
}

#[cfg(test)]
impl FlowGraphView {
    /// How many edges the canvas is holding. The reconcile's own output: a
    /// phantom left beside a reloaded one would show up here as two.
    pub(in crate::workspace) fn drawn_edges_for_test(&self, cx: &gpui::App) -> usize {
        match &self.state {
            FlowGraphState::Graph { canvas, .. } => canvas.read(cx).graph().edges().len(),
            FlowGraphState::Unreadable(_) => 0,
        }
    }
}

#[cfg(test)]
impl FlowGraphView {
    /// Leave the canvas the way the vendored plugin does when a wire is
    /// dropped on empty space and the dangling endpoint is then clicked: a
    /// blank node the flow file has never heard of, plus an edge into it.
    ///
    /// Built here rather than driven through the plugin because what is being
    /// tested is the reconcile's answer to that state, and reaching it for real
    /// needs two mouse gestures at coordinates.
    pub(in crate::workspace) fn strand_a_blank_node_for_test(
        &mut self,
        out_of: &daruda_flow::NodeId,
        cx: &mut Context<Self>,
    ) {
        use crate::ui::flow_canvas::PortKind;

        let FlowGraphState::Graph { canvas, ids, .. } = &self.state else {
            return;
        };
        let Some(out_of) = ids.canvas(out_of) else {
            return;
        };
        let canvas = canvas.clone();
        let source = canvas.read(cx).graph().ports().values().find_map(|port| {
            (port.node_id() == out_of && port.kind() == PortKind::Output).then(|| port.id())
        });
        let Some(source) = source else {
            return;
        };
        canvas.update(cx, |canvas, cx| {
            canvas.dispatch_command(StrandBlankNode { source }, cx);
        });
    }

    /// How many nodes the canvas is holding, for a test asking whether a stray
    /// survived.
    pub(in crate::workspace) fn drawn_nodes_for_test(&self, cx: &gpui::App) -> usize {
        match &self.state {
            FlowGraphState::Graph { canvas, .. } => canvas.read(cx).graph().nodes().len(),
            FlowGraphState::Unreadable(_) => 0,
        }
    }
}

#[cfg(test)]
struct StrandBlankNode {
    source: crate::ui::flow_canvas::PortId,
}

#[cfg(test)]
impl Command for StrandBlankNode {
    fn name(&self) -> &'static str {
        "daruda_test_strand_blank_flow_node"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        // `""` is the type the vendor's own dangling-drop path uses, so the
        // node is as unrenderable and as unnamed as the real one.
        let (node, ports, _) = ctx.create_node("").input().build_with_ports();
        let Some(target) = ports.first().copied() else {
            return;
        };
        let _ = node;
        let edge = ctx.new_edge().source(self.source).target(target);
        ctx.add_edge(edge);
    }

    fn undo(&mut self, _ctx: &mut CommandContext) {}
}
