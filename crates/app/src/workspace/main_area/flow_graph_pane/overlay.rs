//! The `rerun` back-edge — a gate sending nodes back to be re-derived.
//!
//! Drawn here rather than as a graph edge because it is not a dependency:
//! in the flow model it lives inside `GateFail::Repair`, and giving it an
//! `Edge` would put it in the same visual vocabulary as `deps`.
//!
//! **The layer does not do the work.** `RenderLayer::Edges` sits *below*
//! `Nodes`, and the vendored `GraphPlugin` draws both cards and `deps`
//! edges in `Nodes` — so anything painted here is behind them. What keeps
//! the line visible is its route: it drops below the row, runs back, and
//! climbs, so it never crosses a card. A graph shape that forces it to
//! cross would need `Overlay` and card-avoiding routing instead.

use std::collections::HashMap;

use gpui::{
    AnyElement, Element as _, ParentElement as _, PathBuilder, Point, Styled as _, canvas, div, px,
    rgb,
};

use super::model::GraphEdge;
use super::renderer::rgb_u32;
use crate::surface::strings as s;
use crate::ui::flow_canvas::{NodeId, Plugin, RenderContext, RenderLayer};
use crate::ui::theme::palette;

/// How far below the row the curve dips, in world units before zoom.
const DROP: f32 = 58.0;
/// Samples along the curve; every other span is inked, which is the dash.
const SAMPLES: usize = 24;
const STROKE: f32 = 1.4;

pub(super) struct RerunOverlay {
    /// Flow node id → the canvas node it became. The flow model speaks in
    /// the ids written in the file; the canvas assigns its own.
    ids: HashMap<String, NodeId>,
    edges: Vec<GraphEdge>,
    /// Card size, needed to find a card's bottom-centre. The canvas knows
    /// it too, but only per node; this is the one size every card shares.
    card: gpui::Size<f32>,
}

impl RerunOverlay {
    pub(super) fn new(ids: HashMap<String, NodeId>, edges: Vec<GraphEdge>) -> Self {
        Self {
            ids,
            edges,
            card: gpui::Size {
                width: palette::FLOW_GRAPH_NODE_W,
                height: palette::FLOW_GRAPH_NODE_H,
            },
        }
    }
}

impl Plugin for RerunOverlay {
    fn name(&self) -> &'static str {
        "flow_rerun_overlay"
    }

    fn render_layer(&self) -> RenderLayer {
        RenderLayer::Edges
    }

    fn render(&mut self, ctx: &mut RenderContext) -> Option<AnyElement> {
        if self.edges.is_empty() {
            return None;
        }
        let zoom = ctx.zoom();
        let drop = px(DROP * zoom);
        let card = self.card;

        let mut curves = Vec::new();
        let mut labels = Vec::new();
        for edge in &self.edges {
            let (Some(from), Some(to)) = (self.ids.get(&edge.from), self.ids.get(&edge.to)) else {
                continue;
            };
            let (Some(a), Some(b)) = (
                bottom_centre(ctx, *from, card),
                bottom_centre(ctx, *to, card),
            ) else {
                continue;
            };
            let c1 = Point::new(a.x, a.y + drop);
            let c2 = Point::new(b.x, b.y + drop);
            labels.push(Point::new(
                (a.x + b.x) / 2.0,
                (a.y + b.y) / 2.0 + drop * 0.75,
            ));
            curves.push((a, c1, c2, b));
        }
        if curves.is_empty() {
            return None;
        }

        let stroke = rgb(rgb_u32(palette::FLOW_GRAPH_STATUS_RETRIED));
        let painter = canvas(
            move |_, _, _| curves,
            move |bounds, curves, win, _| {
                for (a, c1, c2, b) in curves.iter() {
                    for i in (0..SAMPLES).step_by(2) {
                        let mut seg = PathBuilder::stroke(px(STROKE));
                        seg.move_to(cubic(bounds.origin, *a, *c1, *c2, *b, i, SAMPLES));
                        seg.line_to(cubic(bounds.origin, *a, *c1, *c2, *b, i + 1, SAMPLES));
                        if let Ok(path) = seg.build() {
                            win.paint_path(path, stroke);
                        }
                    }
                }
            },
        );

        let label = s::flow_graph_rerun_edge();
        let mut root = div()
            .absolute()
            .size_full()
            .child(painter.absolute().size_full());
        for at in labels {
            root = root.child(
                div()
                    .absolute()
                    .left(at.x - px(palette::FLOW_GRAPH_NODE_W * 0.1))
                    .top(at.y)
                    .px(px(palette::FLOW_GRAPH_CHIP_PAD_X))
                    .rounded(px(palette::FLOW_GRAPH_CHIP_RADIUS))
                    .bg(rgb(rgb_u32(palette::FLOW_GRAPH_BACKGROUND)))
                    .text_color(rgb(rgb_u32(palette::FLOW_GRAPH_STATUS_RETRIED)))
                    .text_size(px(palette::FLOW_GRAPH_CHIP_FONT_SIZE))
                    .child(label.clone()),
            );
        }
        Some(root.into_any())
    }
}

/// Where the curve leaves and enters a card: the middle of its bottom edge.
fn bottom_centre(
    ctx: &RenderContext,
    id: NodeId,
    card: gpui::Size<f32>,
) -> Option<Point<gpui::Pixels>> {
    let world = ctx.graph.node_world_point(id)?;
    Some(ctx.world_to_screen(Point::new(
        world.x + px(card.width / 2.0),
        world.y + px(card.height),
    )))
}

/// One point on the cubic, offset into the canvas's own bounds.
fn cubic(
    origin: Point<gpui::Pixels>,
    a: Point<gpui::Pixels>,
    c1: Point<gpui::Pixels>,
    c2: Point<gpui::Pixels>,
    b: Point<gpui::Pixels>,
    step: usize,
    of: usize,
) -> Point<gpui::Pixels> {
    let t = step as f32 / of as f32;
    let u = 1.0 - t;
    let at = |pa: f32, p1: f32, p2: f32, pb: f32| {
        u * u * u * pa + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * pb
    };
    Point::new(
        origin.x + px(at(a.x.into(), c1.x.into(), c2.x.into(), b.x.into())),
        origin.y + px(at(a.y.into(), c1.y.into(), c2.y.into(), b.y.into())),
    )
}
