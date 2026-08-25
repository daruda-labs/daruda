//! What the canvas has selected, in the flow's own terms — and putting a
//! selection back after the graph is rebuilt.
//!
//! The canvas's graph is the one thing that knows what was clicked, so nothing
//! here mirrors it; every question is asked of the canvas and translated through
//! [`super::node_ids::NodeIds`].

use daruda_flow::NodeId;
use gpui::{App, Context, Window};

use super::commands::SelectNode;
use super::{FlowGraphState, FlowGraphView, form};

/// What the canvas has selected, in the flow's own terms.
///
/// Three variants rather than `Option<String>` plus a count: a marquee can take
/// several cards, and "several" is a state the inspector has to say something
/// about — it cannot show one node's fields and stay honest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum Selection {
    None,
    One(NodeId),
    /// More than one, by flow node id. The count the inspector shows is
    /// `len()` — an id list gives that *and* what a delete has to act on, which
    /// a count cannot.
    Many(Vec<NodeId>),
}

impl FlowGraphView {
    /// Which flow node the canvas has selected.
    ///
    /// Read from the canvas rather than mirrored here: the canvas's graph is the
    /// one thing that knows what was clicked, and a copy on this side would be a
    /// second answer to the same question.
    pub(in crate::workspace) fn selection(&self, cx: &App) -> Selection {
        let FlowGraphState::Graph { canvas, ids, .. } = &self.state else {
            return Selection::None;
        };
        let selected = canvas.read(cx).graph().selected_node();
        // Ours only, and in the file's order: a canvas node with no flow id is
        // nothing this side can name, and a set has no order to show. Asked of
        // the selection rather than of every node, so panning a large graph
        // does not walk it.
        let mut flow_ids: Vec<NodeId> = selected
            .iter()
            .filter_map(|canvas_id| ids.flow(*canvas_id).cloned())
            .collect();
        flow_ids.sort();
        match flow_ids.len() {
            0 => Selection::None,
            1 => Selection::One(flow_ids.remove(0)),
            _ => Selection::Many(flow_ids),
        }
    }

    /// Every selected node, one or many. What a delete acts on.
    pub(in crate::workspace) fn selected_nodes(&self, cx: &App) -> Vec<NodeId> {
        match self.selection(cx) {
            Selection::None => Vec::new(),
            Selection::One(node) => vec![node],
            Selection::Many(nodes) => nodes,
        }
    }

    /// Select a node so a capture can show the inspector.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn select_node_for_shot(
        &mut self,
        node: &NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_node(node, window, cx);
    }

    /// Select a node the way a click would, for tests that are about what the
    /// selection *does* rather than about hit-testing (which
    /// `clicking_a_card_selects_it_and_does_not_move_it` covers).
    #[cfg(test)]
    pub(in crate::workspace) fn select_node_for_test(
        &mut self,
        node: &NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_node(node, window, cx);
    }
}

impl FlowGraphView {
    /// Follow the canvas's selection: build the form for a newly selected node,
    /// drop it when the selection goes away or grows.
    ///
    /// Does nothing when the selection has not changed — the canvas notifies for
    /// pan, zoom and run colouring too, and rebuilding the form on those would
    /// throw away what the person is typing.
    pub(super) fn reconcile_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = match self.selection(cx) {
            Selection::One(node) => Some(node),
            Selection::None | Selection::Many(_) => None,
        };
        if wanted == self.form.as_ref().map(|form| form.node.clone()) {
            return;
        }
        let text = self.text.clone();
        let issues = self.current_issues();
        self.form = wanted
            .zip(text)
            .and_then(|(node, text)| form::NodeForm::build(&text, &node, &issues, window, cx));
        cx.notify();
    }

    /// Which node is selected, by the flow's own id. `None` when nothing or
    /// several are.
    pub(in crate::workspace) fn selected_node(&self, cx: &App) -> Option<NodeId> {
        match self.selection(cx) {
            Selection::One(node) => Some(node),
            Selection::None | Selection::Many(_) => None,
        }
    }

    /// Select a node that has just been written into the file.
    ///
    /// The write's reload has already rebuilt the graph, so the node exists on
    /// the canvas by now; this is the same path a reload uses to put a selection
    /// back, called with a name that was not selected before.
    pub(in crate::workspace) fn select_node_after_add(
        &mut self,
        node: &NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_node(node, window, cx);
    }

    /// Select `node` on the canvas, if the graph still has it, and build its
    /// form. Used after a reload to put back what was selected before.
    pub(super) fn select_node(
        &mut self,
        node: &NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let FlowGraphState::Graph { canvas, ids, .. } = &self.state else {
            return;
        };
        let Some(canvas_id) = ids.canvas(node) else {
            return;
        };
        canvas.update(cx, |canvas, cx| {
            canvas.dispatch_command(SelectNode { node: canvas_id }, cx);
        });
        self.reconcile_selection(window, cx);
    }
}
