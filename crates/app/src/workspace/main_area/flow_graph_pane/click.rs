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

    /// Above `SelectionPlugin` (100) and `GraphPlugin` (120) is where a marquee
    /// would otherwise win; 130 puts this first among the mouse handlers.
    fn priority(&self) -> i32 {
        130
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
