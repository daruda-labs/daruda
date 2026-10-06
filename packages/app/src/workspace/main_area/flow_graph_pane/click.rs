//! Clicking a card selects it — and does nothing else.
//!
//! The vendored `NodeInteractionPlugin` is what normally turns a click into a
//! selection, and it is deliberately not in daruda's plugin set: its mouse-down
//! handler starts a node **drag** (`plugins/node/interaction.rs`), and a drag
//! carries no information here — the flow file declares dependencies, not
//! positions, so the next reload puts the node back. Selecting without dragging
//! is a small enough policy to own.
//!
//! Above `SelectionPlugin` (100) on purpose: that one starts a marquee on
//! mouse-down anywhere, so a click that lands on a card has to be claimed
//! first or the drag-select would begin on top of it.

use crate::ui::flow_canvas::{EventResult, FlowEvent, InputEvent, Plugin, PluginContext};

pub(super) struct NodeClickPlugin;

impl NodeClickPlugin {
    pub(super) fn new() -> Self {
        Self
    }
}

impl Plugin for NodeClickPlugin {
    fn name(&self) -> &'static str {
        "daruda_node_click"
    }

    /// Above `SelectionPlugin` (100) and `GraphPlugin` (120), where a marquee
    /// would otherwise win — and **below `PortInteractionPlugin` (125)**.
    ///
    /// A port is drawn centred on the card's edge, so half its hit box lies
    /// inside the card. Claiming the press first would swallow that half and
    /// leave the inner semicircle of every port dead.
    fn priority(&self) -> i32 {
        122
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        let FlowEvent::Input(InputEvent::MouseDown(ev)) = event else {
            return EventResult::Continue;
        };
        if ev.button != gpui::MouseButton::Left {
            return EventResult::Continue;
        }
        let world = ctx.screen_to_world(ev.position);
        match ctx.hit_node(world) {
            Some(id) => {
                // `add_selected_node` replaces the selection on its own unless
                // shift is held, in which case it toggles this one.
                ctx.add_selected_node(id, ev.modifiers.shift);
                ctx.notify();
                // Claimed: no marquee over a card, and no drag either.
                EventResult::Stop
            }
            None => {
                // Empty space clears, then falls through so the marquee can
                // still start. Notify only when there was something to clear —
                // gpui has no partial redraw, so an idle click on the
                // background must not cost a frame.
                if !ctx.graph.selected_node_is_empty() {
                    ctx.clear_selected_node();
                    ctx.notify();
                }
                EventResult::Continue
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::flow_canvas::{
        GraphPlugin, PortInteractionPlugin, SelectionPlugin, ViewportPlugin,
    };

    /// The ordering this plugin's number only means something relative to, and
    /// the reason it is not the highest. A port is drawn centred on the card's
    /// edge, so half its hit box is inside the card: claiming the press before
    /// `PortInteractionPlugin` sees it kills the inner half of every port, and
    /// a drag can then only start from outside the card.
    ///
    /// Asserted against the vendor's own numbers rather than written down as
    /// literals, so re-vendoring a changed priority breaks here first.
    #[test]
    fn a_press_on_a_port_reaches_the_port_plugin_first() {
        let ours = NodeClickPlugin::new().priority();
        assert!(
            ours < PortInteractionPlugin::new().priority(),
            "a card must not swallow a press that landed on a port"
        );
        assert!(
            ours > GraphPlugin::new().priority() && ours > SelectionPlugin::new().priority(),
            "but it still beats the marquee and the graph's own handling"
        );
        assert!(ours > ViewportPlugin::new().priority());
    }
}
