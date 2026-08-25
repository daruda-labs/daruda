//! Taking a card away from the keyboard.
//!
//! The neighbour of [`super::disconnect`], and deliberately shaped differently.
//! Removing an edge is a change to the picture that the reconcile later turns
//! into a write; removing a node is a write to the flow file and a question
//! asked first, and neither is a canvas plugin's to do. So this plugin does not
//! touch the graph at all — it records that the key landed, and the view turns
//! that into the same [`FlowGraphEvent::Delete`] the toolbar and the context
//! menu emit. One dialog, one delete path, three ways in.
//!
//! Sits just under [`super::disconnect::EdgeDeletePlugin`] in priority so a
//! Delete with a line selected takes the line and stops there. Reading the edge
//! selection again here would be a second copy of that rule; the ordering is
//! the rule.

use std::cell::Cell;
use std::rc::Rc;

use crate::ui::flow_canvas::{EventResult, FlowEvent, InputEvent, Plugin, PluginContext};

/// The keys that take a card away. Both, because a Mac keyboard's `delete` is
/// the one a Windows keyboard calls backspace.
const DELETE_KEYS: [&str; 2] = ["delete", "backspace"];

/// A Delete or Backspace that landed while cards were selected.
///
/// Shared between the plugin that sees the key and the view that answers it:
/// a plugin runs inside the canvas and cannot reach the pane, and the canvas
/// is rebuilt on every reload — so the flag belongs to the view and the plugin
/// holds a handle to it, the way the rerun overlay holds the model's.
#[derive(Clone, Default)]
pub(super) struct DeleteRequest(Rc<Cell<bool>>);

impl DeleteRequest {
    /// Whether a delete was asked for since the last look, clearing it.
    ///
    /// Taken rather than read: the view is notified for pan, zoom and run
    /// colouring too, and a flag left set would put the dialog up again on the
    /// next unrelated frame.
    pub(super) fn take(&self) -> bool {
        self.0.replace(false)
    }
}

pub(super) struct NodeDeletePlugin {
    asked: DeleteRequest,
}

impl NodeDeletePlugin {
    pub(super) fn new(asked: DeleteRequest) -> Self {
        Self { asked }
    }
}

impl Plugin for NodeDeletePlugin {
    fn name(&self) -> &'static str {
        "daruda_node_delete"
    }

    fn priority(&self) -> i32 {
        85
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        let FlowEvent::Input(InputEvent::KeyDown(ev)) = event else {
            return EventResult::Continue;
        };
        if !DELETE_KEYS.contains(&ev.keystroke.key.as_str()) {
            return EventResult::Continue;
        }
        if ctx.graph.selected_node_is_empty() {
            return EventResult::Continue;
        }
        self.asked.0.set(true);
        // Notify, or nothing wakes the view: this changed no part of the graph,
        // so the canvas has no reason of its own to redraw.
        ctx.notify();
        EventResult::Stop
    }
}

impl DeleteRequest {
    /// Report the key the way the plugin does, for a test about what the view
    /// makes of it. A headless window has no canvas to press a key on.
    #[cfg(test)]
    pub(in crate::workspace) fn ask_for_test(&self) {
        self.0.set(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flag survives being set twice and is gone once read — the view is
    /// notified far more often than a key is pressed.
    #[test]
    fn a_request_is_answered_once() {
        let asked = DeleteRequest::default();
        assert!(!asked.take(), "nothing asked yet");
        asked.0.set(true);
        assert!(asked.take());
        assert!(!asked.take(), "answered already");
    }

    /// The plugin and the view hold the same flag: the canvas is rebuilt on
    /// every reload, so a copy would leave the view watching a dead one.
    #[test]
    fn the_plugin_and_the_view_share_one_flag() {
        let view_side = DeleteRequest::default();
        let plugin = NodeDeletePlugin::new(view_side.clone());
        plugin.asked.0.set(true);
        assert!(view_side.take());
    }
}
