//! Inspection pans the view without moving nodes or changing selection.

use crate::ui::flow_canvas::{
    EventResult, FlowEvent, InputEvent, Interaction, InteractionResult, Plugin, PluginContext,
    ViewportPlugin,
};
use crate::ui::theme::palette;
use gpui::{MouseButton, Pixels, Point, px};

pub(super) struct ExecutionPanPlugin;

fn starts_pan(button: MouseButton, shift: bool, over_node: bool) -> bool {
    button == MouseButton::Middle || (button == MouseButton::Left && (shift || !over_node))
}

impl Plugin for ExecutionPanPlugin {
    fn name(&self) -> &'static str {
        "daruda_execution_pan"
    }
    fn priority(&self) -> i32 {
        130
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        match event {
            FlowEvent::Input(InputEvent::MouseDown(ev)) => {
                let over_node = ctx.hit_node(ctx.screen_to_world(ev.position)).is_some();
                if starts_pan(ev.button, ev.modifiers.shift, over_node) {
                    ctx.start_interaction(Pan {
                        origin: ev.position,
                        offset: ctx.offset(),
                    });
                    return EventResult::Stop;
                }
            }
            FlowEvent::Input(InputEvent::Wheel(ev)) => {
                if ev.modifiers.control || ev.modifiers.platform {
                    ViewportPlugin::new().on_event(event, ctx);
                } else {
                    let delta = ev.delta.pixel_delta(px(palette::FLOW_GRAPH_SCROLL_LINE_PX));
                    ctx.translate_offset(delta.x, delta.y);
                    ctx.notify();
                }
                return EventResult::Stop;
            }
            _ => {}
        }
        EventResult::Continue
    }
}

struct Pan {
    origin: Point<Pixels>,
    offset: Point<Pixels>,
}
impl Interaction for Pan {
    fn on_mouse_move(
        &mut self,
        ev: &gpui::MouseMoveEvent,
        ctx: &mut PluginContext,
    ) -> InteractionResult {
        ctx.set_offset(self.offset + (ev.position - self.origin));
        ctx.notify();
        InteractionResult::Continue
    }
    fn on_mouse_up(
        &mut self,
        _: &gpui::MouseUpEvent,
        ctx: &mut PluginContext,
    ) -> InteractionResult {
        ctx.cancel_interaction();
        ctx.notify();
        InteractionResult::End
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_drag_pans_and_node_clicks_keep_their_selection_behavior() {
        assert!(starts_pan(MouseButton::Left, false, false));
        assert!(!starts_pan(MouseButton::Left, false, true));
        assert!(starts_pan(MouseButton::Left, true, true));
        assert!(starts_pan(MouseButton::Middle, false, true));
        assert!(!starts_pan(MouseButton::Right, false, false));
        assert!(
            ExecutionPanPlugin.priority() > super::super::click::NodeClickPlugin::new().priority()
        );
    }
}
