//! The two edits this pane sends to the canvas.
//!
//! Commands because the canvas hands out its graph immutably and takes writes
//! only this way — there is no `graph_mut`. `undo` exists for the trait's sake
//! and is never reached: the pane installs no `HistoryPlugin`, so nothing is
//! bound to run it. The YAML file is the only undo stack.

use crate::ui::flow_canvas::{CanvasNodeId, Command, CommandContext};

/// Write fresh card data onto nodes that already exist.
pub(super) struct StampCards {
    pub(super) cards: Vec<(CanvasNodeId, serde_json::Value)>,
}

impl Command for StampCards {
    fn name(&self) -> &'static str {
        "daruda_stamp_flow_cards"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        for (id, data) in &self.cards {
            if let Some(node) = ctx.graph.get_node_mut(id) {
                node.set_data(data.clone());
            }
        }
    }

    fn undo(&mut self, _ctx: &mut CommandContext) {}
}

/// Put the selection back on a node after the graph was rebuilt.
pub(super) struct SelectNode {
    pub(super) node: CanvasNodeId,
}

impl Command for SelectNode {
    fn name(&self) -> &'static str {
        "daruda_select_flow_node"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        ctx.add_selected_node(self.node, false);
    }

    fn undo(&mut self, _ctx: &mut CommandContext) {}
}
