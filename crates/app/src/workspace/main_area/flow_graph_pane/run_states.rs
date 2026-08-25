//! Painting a run onto the cards.
//!
//! The one place a run reaches the drawing. What the states mean lives in
//! [`super::model`], and the cards themselves are [`super::renderer`]'s — this
//! only decides whether this graph is the one the run is about.

use gpui::Context;

use super::commands::StampCards;
use super::model::RunColouring;
use super::renderer::card_for;
use super::{FlowGraphState, FlowGraphView};
use crate::ui::flow_canvas::CanvasNodeId;

impl FlowGraphView {
    /// Colour the cards from what a run has reported so far.
    ///
    /// Only the cards change — the graph's shape came from the file and a run
    /// cannot alter it, so nothing is re-laid-out and nothing moves under the
    /// person's eyes as a run progresses.
    ///
    /// Nothing is painted when `colouring` is about nodes the file no longer
    /// has: a run executes the flow it resolved at the start, and an id freed by
    /// a delete and taken by a rename would otherwise wear the first node's
    /// colour.
    ///
    /// What makes it visible past this view's `.cached()` wrapper is a notify on
    /// the *canvas*: `execute_command` raises one itself (`plugin.rs`), and the
    /// one below is ours rather than borrowed, so a future stamp that stops
    /// going through a `Command` does not silently lose the repaint.
    pub(in crate::workspace) fn set_run_states(
        &mut self,
        colouring: &RunColouring,
        cx: &mut Context<Self>,
    ) {
        let FlowGraphState::Graph { canvas, model, ids } = &self.state else {
            return;
        };
        if !colouring.is_about(model.nodes.iter().map(|node| node.id.clone())) {
            return;
        }
        // Read before the state below is borrowed, and kept: this is called to
        // put a *previous* run's colours back after a reload, which is not the
        // person having acted on anything. Blanking it here erased the reason
        // one frame after an edit produced it, whenever the flow had ever run.
        let unpinned = self.unpinned.clone();
        let pins = self.pins.clone();
        let stamped: Vec<(CanvasNodeId, serde_json::Value)> = model
            .nodes
            .iter()
            .filter_map(|node| {
                let id = ids.canvas(&node.id)?;
                let state = colouring.states.get(&node.id).copied().unwrap_or_default();
                let card = card_for(
                    node,
                    super::renderer::CardFacts {
                        run: state,
                        pinned: pins.contains(&node.id),
                        issues: model.issues_naming(&node.id),
                        unpinned: unpinned
                            .iter()
                            .find(|(id, _)| id == &node.id)
                            .map(|(_, why)| why),
                    },
                );
                Some((id, serde_json::to_value(&card).unwrap_or_default()))
            })
            .collect();
        canvas.update(cx, |canvas, cx| {
            canvas.dispatch_command(StampCards { cards: stamped }, cx);
            cx.notify();
        });
    }
}
