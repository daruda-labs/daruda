//! Putting the graph in the pane — daruda's own framing policy.
//!
//! The vendored `FitAllGraphPlugin` does this too, but with upstream's
//! numbers: it magnifies up to 3× and refuses to go below 0.7. Both are
//! decisions about *cards*, and daruda's cards decide differently — they drop
//! rows as they shrink (`renderer::CardDensity`), so how far out it is worth
//! zooming is a question only this side can answer. Owning the policy here
//! also means one rule for both entry points, opening and ⌘0, instead of a
//! graph that frames one way on open and another when asked again.
//!
//! The vendor is reached only through public API: a `Plugin` for the two
//! events, and a `Command` for the write — `PluginContext` has no public
//! viewport setter, `CommandContext` does.

use gpui::{Point, px};

use crate::ui::flow_canvas::{
    Command, CommandContext, EventResult, FlowEvent, InputEvent, Plugin, PluginContext,
    primary_platform_modifier,
};
use crate::ui::theme::palette;

/// Where the graph should sit, in the viewport's own terms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Framing {
    pub zoom: f32,
    pub offset: Point<f32>,
}

/// Fit `graph` (world-space `x, y, w, h`) into a `drawable_w × drawable_h`
/// pane: centred, scaled down far enough to get all of it in, and never
/// scaled up.
///
/// Magnification is refused rather than clamped away: a graph smaller than
/// the pane is already all in view, and a card's text does not grow with its
/// box, so enlarging one only spreads the same words over a bigger rectangle.
pub(super) fn frame_for(
    drawable_w: f32,
    drawable_h: f32,
    graph: (f32, f32, f32, f32),
) -> Option<Framing> {
    if drawable_w <= 0.0 || drawable_h <= 0.0 {
        return None;
    }
    let (gx, gy, gw, gh) = graph;
    let (gw, gh) = (gw.max(1.0), gh.max(1.0));

    let usable = 1.0 - 2.0 * palette::FLOW_GRAPH_FRAME_MARGIN;
    let zoom = (drawable_w * usable / gw)
        .min(drawable_h * usable / gh)
        .clamp(palette::FLOW_GRAPH_FRAME_MIN_ZOOM, 1.0);

    // Centre of the graph onto the centre of the pane.
    Some(Framing {
        zoom,
        offset: Point::new(
            drawable_w / 2.0 - (gx + gw / 2.0) * zoom,
            drawable_h / 2.0 - (gy + gh / 2.0) * zoom,
        ),
    })
}

/// Write a [`Framing`] to the viewport. A command because that is the only
/// public way in; never undone (the pane installs no history plugin).
struct SetFraming(Framing);

impl Command for SetFraming {
    fn name(&self) -> &'static str {
        "daruda_frame_flow_graph"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        ctx.set_zoom(self.0.zoom);
        ctx.set_offset(Point::new(px(self.0.offset.x), px(self.0.offset.y)));
    }

    fn undo(&mut self, _ctx: &mut CommandContext) {}
}

/// How much the drawable has to change before it counts as a new size rather
/// than layout noise.
const WIDTH_CHANGE_EPSILON: f32 = 1.0;

/// Frames the graph when the pane's size is known, again when that size
/// changes, and again on ⌘0.
pub(super) struct FrameGraphPlugin {
    /// The width the graph was last framed into. `window_bounds` is re-reported
    /// on **every** prepaint, so framing on each of those would fight any pan the
    /// person has done since; framing when the width actually changes is the
    /// narrower rule. It is also the one the inspector needs — opening it takes
    /// space away from the canvas, and a graph framed for the old width would
    /// then sit half outside it.
    ///
    /// The cost, stated: a manual pan or zoom is lost when the pane's width
    /// changes. ⌘0 puts it back, and a graph nobody can see is worse.
    framed_at: Option<f32>,
}

impl FrameGraphPlugin {
    pub(super) fn new() -> Self {
        Self { framed_at: None }
    }

    /// Has the drawable a width this plugin has not framed for yet?
    fn width_is_new(&self, width: f32) -> bool {
        self.framed_at
            .is_none_or(|framed| (framed - width).abs() > WIDTH_CHANGE_EPSILON)
    }

    fn frame(&self, ctx: &mut PluginContext) -> bool {
        let Some(bounds) = ctx.window_bounds() else {
            return false;
        };
        let Some(graph) = ctx.graph.nodes_world_aabb() else {
            return false;
        };
        let Some(framing) = frame_for(
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
            graph,
        ) else {
            return false;
        };
        ctx.execute_command(SetFraming(framing));
        true
    }
}

impl Plugin for FrameGraphPlugin {
    fn name(&self) -> &'static str {
        "daruda_frame_graph"
    }

    /// Below `GraphPlugin` (120) and `SelectionPlugin` (100), matching where
    /// the vendored fit-all sat: this handles a key nothing else claims, and
    /// should see it after anything that might.
    fn priority(&self) -> i32 {
        88
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        match event {
            FlowEvent::DrawableBoundsReady => {
                let width = ctx.window_bounds().map(|b| f32::from(b.size.width));
                if let Some(width) = width
                    && self.width_is_new(width)
                    && self.frame(ctx)
                {
                    self.framed_at = Some(width);
                }
                EventResult::Continue
            }
            FlowEvent::Input(InputEvent::KeyDown(ev))
                if primary_platform_modifier(ev)
                    && !ev.keystroke.modifiers.shift
                    && ev.keystroke.key == "0" =>
            {
                self.frame(ctx);
                EventResult::Stop
            }
            _ => EventResult::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pane the size the flow pane actually gets, measured from a capture.
    const PANE_W: f32 = 795.0;
    const PANE_H: f32 = 660.0;

    /// World extent of a chain of `n` cards at the layout's own pitch
    /// (`card width + layer_spacing`, the vendored default being 120).
    fn chain(n: usize) -> (f32, f32, f32, f32) {
        let pitch = palette::FLOW_GRAPH_NODE_W + 120.0;
        (
            0.0,
            0.0,
            pitch * (n as f32 - 1.0) + palette::FLOW_GRAPH_NODE_W,
            palette::FLOW_GRAPH_NODE_H,
        )
    }

    /// The rule the inspector depends on: the same width does not re-frame, a
    /// different one does. Without it, opening the inspector would leave the
    /// graph framed for a canvas that is no longer that wide.
    #[test]
    fn only_a_width_it_has_not_framed_for_counts() {
        let mut plugin = FrameGraphPlugin::new();
        assert!(plugin.width_is_new(PANE_W), "nothing framed yet");
        plugin.framed_at = Some(PANE_W);
        assert!(!plugin.width_is_new(PANE_W), "the same width is not new");
        assert!(
            !plugin.width_is_new(PANE_W + WIDTH_CHANGE_EPSILON / 2.0),
            "layout noise is not a new width"
        );
        assert!(
            plugin.width_is_new(PANE_W - 260.0),
            "an inspector's worth of width is"
        );
    }

    /// A graph that already fits is left at its own size and centred — the
    /// rule that keeps one card from being blown up to fill the pane.
    #[test]
    fn a_graph_smaller_than_the_pane_is_centred_at_its_own_size() {
        let framing = frame_for(PANE_W, PANE_H, chain(1)).expect("a sized pane frames");
        assert_eq!(framing.zoom, 1.0);
        assert!(
            framing.offset.x > 0.0 && framing.offset.y > 0.0,
            "a centred graph sits in from the corner: {:?}",
            framing.offset
        );
    }

    /// The case the framing exists for: a chain wider than the pane comes
    /// down to where all of it is inside.
    #[test]
    fn a_chain_wider_than_the_pane_is_scaled_until_it_fits() {
        for n in [4, 6, 8, 10] {
            let graph = chain(n);
            let framing = frame_for(PANE_W, PANE_H, graph).expect("a sized pane frames");
            assert!(framing.zoom < 1.0, "{n} nodes should have been scaled down");
            let right = framing.offset.x + (graph.0 + graph.2) * framing.zoom;
            assert!(
                framing.offset.x >= 0.0 && right <= PANE_W,
                "{n} nodes still overflow: 0..{PANE_W} vs {}..{right}",
                framing.offset.x
            );
        }
    }

    /// Past the floor the graph is *not* framed whole, and that is the
    /// decision: a card narrower than its own id says nothing, so panning is
    /// the better answer than a wall of slivers.
    #[test]
    fn a_graph_past_the_floor_stops_at_the_floor() {
        let framing = frame_for(PANE_W, PANE_H, chain(40)).expect("a sized pane frames");
        assert_eq!(framing.zoom, palette::FLOW_GRAPH_FRAME_MIN_ZOOM);
    }

    /// A pane with no area yet (first layout pass) has nothing to frame into.
    #[test]
    fn an_unmeasured_pane_frames_nothing() {
        assert!(frame_for(0.0, 0.0, chain(3)).is_none());
    }
}
