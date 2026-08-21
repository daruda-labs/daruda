//! Taking a line away.
//!
//! Two affordances, one thing done: **the selected edge comes off the canvas.**
//! Delete or Backspace does it from the keyboard, the pane's context menu does
//! it from the mouse, and neither writes anything. The write is
//! [`super::port_drag`]'s reconcile noticing that the file declares a
//! dependency the picture no longer draws — the same invariant that turns a
//! drawn line into a dependency, read the other way.
//!
//! Funnelled deliberately. Two paths that each edited the file would be two
//! places to get the direction wrong, and the reconcile is where that direction
//! is already decided and tested.
//!
//! Selecting the edge first is the gesture, rather than acting on whatever the
//! pointer is over. `GraphPlugin` — which daruda does attach — already selects
//! an edge on click and draws it in the accent colour, so what would be removed
//! is visible before the key is pressed. It is also what the menu's own
//! neighbour does: "delete node" acts on the selected node, not on the one
//! under the right-click.

use gpui::Context;

use crate::ui::flow_canvas::{
    Command, CommandContext, EdgeId, EventResult, FlowEvent, InputEvent, Plugin, PluginContext,
};

use super::{FlowGraphState, FlowGraphView};

/// The keys that take a line away. Both, because a Mac keyboard's `delete` is
/// the one a Windows keyboard calls backspace.
const DELETE_KEYS: [&str; 2] = ["delete", "backspace"];

pub(super) struct EdgeDeletePlugin;

impl Plugin for EdgeDeletePlugin {
    fn name(&self) -> &'static str {
        "daruda_edge_delete"
    }

    /// Below the mouse handlers and above nothing in particular: it claims a
    /// key no other plugin in daruda's set looks at, and only when an edge is
    /// selected — so a Delete with a node selected still falls through.
    fn priority(&self) -> i32 {
        86
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        let FlowEvent::Input(InputEvent::KeyDown(ev)) = event else {
            return EventResult::Continue;
        };
        if !DELETE_KEYS.contains(&ev.keystroke.key.as_str()) {
            return EventResult::Continue;
        }
        let selected: Vec<EdgeId> = ctx.graph.selected_edge().iter().copied().collect();
        if selected.is_empty() {
            return EventResult::Continue;
        }
        ctx.execute_command(DropSelectedEdges { edges: selected });
        EventResult::Stop
    }
}

/// Take the named edges off the canvas, and forget they were selected — a
/// selection pointing at an edge that no longer exists would keep the menu
/// offering to remove it.
struct DropSelectedEdges {
    edges: Vec<EdgeId>,
}

impl Command for DropSelectedEdges {
    fn name(&self) -> &'static str {
        "daruda_drop_selected_flow_edges"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        for edge in &self.edges {
            ctx.remove_edge(edge);
        }
        ctx.graph.clear_selected_edge();
    }

    fn undo(&mut self, _ctx: &mut CommandContext) {}
}

impl FlowGraphView {
    /// Whether a line is selected, so the menu can offer to remove it.
    pub(in crate::workspace) fn has_selected_edge(&self, cx: &gpui::App) -> bool {
        match &self.state {
            FlowGraphState::Graph { canvas, .. } => {
                !canvas.read(cx).graph().selected_edge().is_empty()
            }
            FlowGraphState::Unreadable(_) => false,
        }
    }

    /// The menu's half of the same gesture the key performs.
    pub(in crate::workspace) fn drop_selected_edges(&mut self, cx: &mut Context<Self>) {
        let FlowGraphState::Graph { canvas, .. } = &self.state else {
            return;
        };
        let edges: Vec<EdgeId> = canvas
            .read(cx)
            .graph()
            .selected_edge()
            .iter()
            .copied()
            .collect();
        if edges.is_empty() {
            return;
        }
        let canvas = canvas.clone();
        canvas.update(cx, |canvas, cx| {
            canvas.dispatch_command(DropSelectedEdges { edges }, cx);
        });
    }
}

#[cfg(test)]
impl FlowGraphView {
    /// Select every line, the way clicking one does — for a test that is about
    /// what removal does rather than about hit-testing a bezier.
    pub(in crate::workspace) fn select_every_edge_for_test(&mut self, cx: &mut Context<Self>) {
        let FlowGraphState::Graph { canvas, .. } = &self.state else {
            return;
        };
        let canvas = canvas.clone();
        let all: Vec<EdgeId> = canvas.read(cx).graph().edges().keys().copied().collect();
        canvas.update(cx, |canvas, cx| {
            canvas.dispatch_command(SelectEdges { edges: all }, cx);
        });
    }

    /// Select the one line between two named cards.
    pub(in crate::workspace) fn select_edge_for_test(
        &mut self,
        out_of: &daruda_flow::NodeId,
        into: &daruda_flow::NodeId,
        cx: &mut Context<Self>,
    ) {
        let FlowGraphState::Graph { canvas, ids, .. } = &self.state else {
            return;
        };
        let (Some(out_of), Some(into)) = (ids.canvas(out_of), ids.canvas(into)) else {
            return;
        };
        let canvas = canvas.clone();
        let found: Vec<EdgeId> = {
            let graph = canvas.read(cx).graph();
            graph
                .edges()
                .values()
                .filter(|edge| {
                    let (Some(s), Some(t)) = (
                        graph.get_port(&edge.source_port),
                        graph.get_port(&edge.target_port),
                    ) else {
                        return false;
                    };
                    s.node_id() == out_of && t.node_id() == into
                })
                .map(|edge| edge.id)
                .collect()
        };
        canvas.update(cx, |canvas, cx| {
            canvas.dispatch_command(SelectEdges { edges: found }, cx);
        });
    }
}

#[cfg(test)]
struct SelectEdges {
    edges: Vec<EdgeId>,
}

#[cfg(test)]
impl Command for SelectEdges {
    fn name(&self) -> &'static str {
        "daruda_test_select_flow_edges"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        ctx.graph
            .set_selected_edge(self.edges.iter().copied().collect());
    }

    fn undo(&mut self, _ctx: &mut CommandContext) {}
}
