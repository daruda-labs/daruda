//! Test and capture hooks on the flow graph view — canvas access, scripted
//! input, and drawn-card inspection.

#[cfg(test)]
use std::collections::HashMap;

#[cfg(test)]
use gpui::{App, Context, Entity, Window};

#[cfg(any(test, feature = "screenshot"))]
use daruda_flow::NodeId;

#[cfg(test)]
use crate::ui::flow_canvas::FlowCanvas;

#[cfg(any(test, feature = "screenshot"))]
use super::FlowGraphState;
use super::FlowGraphView;
#[cfg(test)]
use super::{FlowGraphError, renderer};

impl FlowGraphView {
    /// The canvas entity, for tests that assert where the graph ended up on
    /// screen. `None` when the flow did not load.
    #[cfg(test)]
    pub(in crate::workspace) fn canvas_for_test(&self) -> Option<&Entity<FlowCanvas>> {
        match &self.state {
            FlowGraphState::Graph { canvas, .. } => Some(canvas),
            FlowGraphState::Unreadable(_) => None,
        }
    }

    #[cfg(test)]
    pub(in crate::workspace) fn set_name_for_test(
        &self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.name_input
            .update(cx, |input, cx| input.set_value(name, window, cx));
    }

    /// The flow's node ids in the order the file declares them — what a
    /// scripted capture walks to put one card in each state.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn node_ids_for_shot(&self) -> Vec<NodeId> {
        match &self.state {
            FlowGraphState::Graph { model, .. } => {
                model.nodes.iter().map(|n| n.id.clone()).collect()
            }
            FlowGraphState::Unreadable(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
impl FlowGraphView {
    /// A canvas notify with no key behind it — pan, zoom, run colouring.
    pub(in crate::workspace) fn notified_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_canvas_notified(window, cx);
    }

    /// Move the pointer onto or off the canvas as far as the plugin gets.
    pub(in crate::workspace) fn hover_canvas_for_test(
        &mut self,
        over: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pan_armed.set_hovering_for_test(over);
        self.on_canvas_notified(window, cx);
    }

    /// Hold or release the pan key as far as the canvas plugin gets: it writes
    /// the shared state and the next notify is what the view makes of it.
    pub(in crate::workspace) fn hold_pan_key_for_test(
        &mut self,
        held: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pan_armed.set_held_for_test(held);
        self.on_canvas_notified(window, cx);
    }

    /// What the pane would draw the pointer as.
    pub(in crate::workspace) fn pan_cursor_for_test(
        &self,
    ) -> Option<crate::ui::cursor::CursorReach> {
        self.pan_cursor_shown
    }

    /// Press Delete on the selected cards, as far as the canvas plugin gets:
    /// it records the key and the next notify is what answers it.
    pub(in crate::workspace) fn press_delete_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_request.ask_for_test();
        self.on_canvas_notified(window, cx);
    }

    /// Each node's card as the canvas actually holds it, keyed by flow node id
    /// and reduced to the two fields a run changes. Reading it back through the
    /// canvas is the point: it proves the stamp reached the graph the renderer
    /// draws from, not just the states the workspace accumulated.
    pub(in crate::workspace) fn cards_for_test(
        &self,
        cx: &App,
    ) -> HashMap<NodeId, (String, String)> {
        let FlowGraphState::Graph { canvas, ids, .. } = &self.state else {
            return HashMap::new();
        };
        let graph = canvas.read(cx).graph();
        ids.iter()
            .filter_map(|(flow_id, node_id)| {
                let node = graph.nodes().get(&node_id)?;
                let card: renderer::CardData =
                    serde_json::from_value(node.data_ref().clone()).ok()?;
                Some((flow_id.clone(), (card.badge, format!("{:?}", card.accent))))
            })
            .collect()
    }

    /// How many rules each card says its node breaks.
    pub(in crate::workspace) fn card_issues_for_test(&self, cx: &App) -> HashMap<NodeId, usize> {
        let FlowGraphState::Graph { canvas, ids, .. } = &self.state else {
            return HashMap::new();
        };
        let graph = canvas.read(cx).graph();
        ids.iter()
            .filter_map(|(flow_id, node_id)| {
                let node = graph.nodes().get(&node_id)?;
                let card: renderer::CardData =
                    serde_json::from_value(node.data_ref().clone()).ok()?;
                Some((flow_id.clone(), card.issues))
            })
            .collect()
    }

    /// Why the pane is showing prose instead of a graph. `None` when it drew.
    pub(in crate::workspace) fn unreadable_for_test(&self) -> Option<&FlowGraphError> {
        match &self.state {
            FlowGraphState::Graph { .. } => None,
            FlowGraphState::Unreadable(err) => Some(err),
        }
    }
}
