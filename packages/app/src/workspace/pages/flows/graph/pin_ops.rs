//! Pin controls on the flow graph view — what the pin button would do, the
//! pinned set for a run, and dropping or toggling pins.

use gpui::{App, Context};

use daruda_flow::NodeId;

use super::model::GraphNodeKind;

use super::{FlowGraphState, FlowGraphView, pins};

impl FlowGraphView {
    /// What one press of the pin button would do, given what is selected.
    pub(in crate::workspace) fn pin_action(&self, cx: &App) -> pins::PinAction {
        self.pins.action_for(self.reusable_selection(cx))
    }

    /// Every pinned node, for the run about to be submitted.
    pub(in crate::workspace) fn pinned_nodes(&self) -> Vec<NodeId> {
        self.pins.to_vec()
    }

    /// Pin the selection, or unpin it, and write the cards again so the graph
    /// says which. A run's colours are not put back here — the caller does
    /// that, exactly as it does after a reload, because the run's state lives
    /// on the workspace and not on this view.
    /// Forget pins the run could not honour.
    ///
    /// A pin whose source run has been swept is gone for good, so leaving it
    /// set drew the card as reused on every later press while the node was
    /// quietly paid for again. The one warning that said so is long dismissed
    /// by then; the card is the only thing still on screen.
    pub(in crate::workspace) fn drop_pins(&mut self, nodes: &[NodeId], cx: &mut Context<Self>) {
        let before = self.pins.clone();
        for node in nodes {
            self.pins.remove(node);
        }
        if self.pins == before {
            return;
        }
        // Said on the card for the same reason an edit's drop is: the toast is
        // gone in a moment and the node is on screen the whole time.
        self.unpinned = nodes
            .iter()
            .filter(|node| before.contains(node))
            .map(|node| (node.clone(), pins::PinDropped::SourceGone))
            .collect();
        {}
        let FlowGraphState::Graph { model, .. } = &self.state else {
            return;
        };
        let model = model.clone();
        self.restamp(&model, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn toggle_pins(&mut self, cx: &mut Context<Self>) {
        let nodes = self.reusable_selection(cx);
        if nodes.is_empty() {
            return;
        }
        self.pins.toggle(&nodes);
        let FlowGraphState::Graph { model, .. } = &self.state else {
            return;
        };
        let model = model.clone();
        self.restamp(&model, cx);
        cx.notify();
    }

    /// The selected nodes that have an output to reuse — the agent ones. A gate
    /// writes nothing, so pinning one would name a file that does not exist and
    /// the engine would refuse the whole run over it.
    fn reusable_selection(&self, cx: &App) -> Vec<NodeId> {
        let FlowGraphState::Graph { model, .. } = &self.state else {
            return Vec::new();
        };
        let selected = self.selected_nodes(cx);
        model
            .nodes
            .iter()
            .filter(|node| matches!(node.kind, GraphNodeKind::Agent { .. }))
            .map(|node| node.id.clone())
            .filter(|id| selected.contains(id))
            .collect()
    }
}
