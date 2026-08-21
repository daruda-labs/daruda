//! How a flow node becomes a card, and how that card is drawn.
//!
//! Split from `model.rs` on the wording line: the model carries data, this
//! carries appearance and the strings the user reads. It is also the only
//! file here that touches the vendored canvas types, which it reaches
//! through `crate::ui::flow_canvas` like every other vendored widget.

use gpui::{
    AnyElement, Element as _, Hsla, IntoElement, ParentElement as _, Styled as _, div, px, rgb,
};
use serde::{Deserialize, Serialize};

use super::model::{FailPolicy, GraphNode, GraphNodeKind, NodeRunState, PromptSummary};
use crate::surface::strings as s;
use crate::ui::flow_canvas::{FlowTheme, Node, NodeRenderer, Port, RenderContext};
use crate::ui::theme::{PaneSurfaceTokens, palette};

/// The node type every flow node registers under.
///
/// Not `""` and not `"default"`: the vendored registry short-circuits both
/// to its own built-in renderer, so a custom one registered under either is
/// silently ignored and every card comes out blank.
pub(super) const NODE_TYPE: &str = "flow";

/// What the view writes onto a graph node and this file reads back.
///
/// The canvas owns the graph, so a card's contents travel as the node's
/// `data`. Strings are already resolved here rather than at paint time —
/// the renderer is handed a card, not a model plus a locale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CardData {
    pub id: String,
    /// Agent or gate, as data. The chip below is the same fact worded for
    /// display, and a localized string is not a discriminant — two locales
    /// whose labels collide would silently lose the shape that tells them
    /// apart, and shape is what carries the kind once the chip stops fitting.
    #[serde(default)]
    pub kind: CardKind,
    /// `AGENT` / `GATE`.
    pub chip: String,
    /// Agent axes, or the command a gate runs.
    pub meta: String,
    /// Prompt first line, output path — whatever the one spare line is for.
    pub summary: String,
    /// Run status, or the failure policy when no run is driving this node.
    pub badge: String,
    pub accent: CardAccent,
}

/// What sort of node a card is drawn for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(super) enum CardKind {
    #[default]
    Agent,
    Gate,
}

/// Which colour the card's border and badge take. An enum rather than a
/// packed colour so the palette stays the only place a colour is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum CardAccent {
    Pending,
    Running,
    Passed,
    Retried,
    Failed,
}

impl CardAccent {
    fn hsla(self) -> Hsla {
        match self {
            Self::Pending => palette::FLOW_GRAPH_STATUS_PENDING,
            Self::Running => palette::FLOW_GRAPH_STATUS_RUNNING,
            Self::Passed => palette::FLOW_GRAPH_STATUS_PASSED,
            Self::Retried => palette::FLOW_GRAPH_STATUS_RETRIED,
            Self::Failed => palette::FLOW_GRAPH_STATUS_FAILED,
        }
    }
}

/// Build the card for one node under one run state.
pub(super) fn card_for(node: &GraphNode, run: NodeRunState) -> CardData {
    let kind = match &node.kind {
        GraphNodeKind::Agent { .. } => CardKind::Agent,
        GraphNodeKind::Gate { .. } => CardKind::Gate,
    };
    let (chip, meta, summary) = match &node.kind {
        GraphNodeKind::Agent { agent, prompt, .. } => {
            let mut axes = vec![agent.id.clone()];
            axes.extend(agent.model.clone());
            axes.extend(agent.effort.clone());
            axes.extend(agent.mode.clone());
            axes.push(timeout_label(node.timeout));
            (
                s::flow_graph_kind_agent(),
                axes.join(" · "),
                match prompt {
                    PromptSummary::Inline(line) => line.clone(),
                    PromptSummary::File(path) => path.display().to_string(),
                },
            )
        }
        GraphNodeKind::Gate { run: line } => (
            s::flow_graph_kind_gate(),
            timeout_label(node.timeout),
            line.clone(),
        ),
    };
    let (badge, accent) = badge_for(node, run);
    CardData {
        id: node.id.clone().into_string(),
        kind,
        chip,
        meta,
        summary,
        badge,
        accent,
    }
}

/// `10m`, `1h 30m` — the same vocabulary the flow file is written in.
fn timeout_label(timeout: std::time::Duration) -> String {
    humantime::format_duration(timeout).to_string()
}

/// A card says what the run is doing to this node; with no run to report,
/// it says what the node would do to itself on failure. Both answer the
/// same question — what happens next — so they share one line.
fn badge_for(node: &GraphNode, run: NodeRunState) -> (String, CardAccent) {
    match run {
        NodeRunState::Running { attempt } if attempt > 1 => {
            (s::flow_graph_status_attempt(attempt), CardAccent::Retried)
        }
        NodeRunState::Running { .. } => (s::flow_graph_status_running(), CardAccent::Running),
        NodeRunState::Passed => (s::flow_graph_status_passed(), CardAccent::Passed),
        NodeRunState::Failed => (s::flow_graph_status_failed(), CardAccent::Failed),
        NodeRunState::Fixing => (s::flow_graph_status_fixing(), CardAccent::Retried),
        NodeRunState::Pending => match node.fail {
            FailPolicy::Halt => (String::new(), CardAccent::Pending),
            FailPolicy::Retry { max_attempts } => (
                s::flow_graph_policy_retry(max_attempts),
                CardAccent::Pending,
            ),
            FailPolicy::Repair { max_attempts, .. } => (
                s::flow_graph_policy_repair(max_attempts),
                CardAccent::Pending,
            ),
        },
    }
}

/// The canvas chrome, on the terminal colour theme. `FlowTheme` is
/// `#[non_exhaustive]`, so it is built by mutating a default.
///
/// [`PaneSurfaceTokens`] rather than the UI theme, for the reason agent chat and
/// the file viewer already use it: a content pane sitting on workspace-chrome
/// colours does not match the terminal beside it.
pub(super) fn flow_theme(tokens: &PaneSurfaceTokens) -> FlowTheme {
    let mut t = FlowTheme::default();
    t.background = rgb_u32(tokens.background);
    t.background_grid_dot = over(tokens.border_tint, tokens.background);
    t.edge_stroke = rgb_u32(tokens.foreground_muted_over_background());
    t.edge_stroke_selected = rgb_u32(palette::FLOW_GRAPH_STATUS_RUNNING);
    t.default_port_fill = rgb_u32(tokens.foreground_muted_over_background());
    // The wire being dragged between two ports, refused and accepted. Taken
    // from the run's own status colours rather than picked fresh: a card that
    // passed is already this green, and one that failed already this red, so a
    // drag says yes and no in the vocabulary the graph has been using.
    t.error = rgb_u32(palette::FLOW_GRAPH_STATUS_FAILED);
    t.success = rgb_u32(palette::FLOW_GRAPH_STATUS_PASSED);
    t
}

/// The colours a card is drawn with, resolved once when the canvas is built.
///
/// Carried by the renderer because a [`NodeRenderer`] sees neither an `App` nor
/// the canvas's shared state — `RenderContext` gives it a `Window`, a
/// `FlowTheme` and the graph, and nothing else. The renderer is daruda's own, so
/// this is the one place the current theme can reach a card.
#[derive(Clone, Copy)]
pub(super) struct CardPalette {
    pub card_bg: u32,
    /// A node nothing has happened to yet. A surface tone rather than a status
    /// hue, so unlike the other four it has to follow the theme.
    pub pending_border: u32,
    pub chip_bg: u32,
    pub text_body: u32,
    pub text_mute: u32,
    pub text_subtle: u32,
}

impl CardPalette {
    pub(super) fn of(tokens: &PaneSurfaceTokens) -> Self {
        Self {
            card_bg: over(tokens.tint, tokens.background),
            pending_border: over(tokens.border_tint, tokens.background),
            chip_bg: over(tokens.active_tint, tokens.background),
            text_body: rgb_u32(tokens.foreground),
            text_mute: rgb_u32(tokens.foreground_muted_over_background()),
            text_subtle: rgb_u32(tokens.foreground_subtle_over_background()),
        }
    }
}

/// Flatten `top` onto `bottom` and pack the result.
///
/// The canvas takes opaque `0x00RRGGBB` (see [`FlowTheme`]), while the pane
/// tokens are overlays carrying alpha — a card tint is 6% of a neutral over the
/// surface. Dropping the alpha would paint that neutral at full strength, so it
/// is composited here instead.
fn over(top: Hsla, bottom: Hsla) -> u32 {
    let (top, bottom) = (gpui::Rgba::from(top), gpui::Rgba::from(bottom));
    let a = top.a.clamp(0.0, 1.0);
    let mix = |t: f32, b: f32| t * a + b * (1.0 - a);
    let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    (to_byte(mix(top.r, bottom.r)) << 16)
        | (to_byte(mix(top.g, bottom.g)) << 8)
        | to_byte(mix(top.b, bottom.b))
}

/// The muted / subtle text ramp, already flattened onto the surface.
///
/// `PaneSurfaceTokens` dims by alpha, which a canvas cannot draw — so the same
/// dimming is computed against the pane's own background here.
trait FlattenedRamp {
    fn foreground_muted_over_background(&self) -> Hsla;
    fn foreground_subtle_over_background(&self) -> Hsla;
}

impl FlattenedRamp for PaneSurfaceTokens {
    fn foreground_muted_over_background(&self) -> Hsla {
        flatten(self.foreground_muted, self.background)
    }

    fn foreground_subtle_over_background(&self) -> Hsla {
        flatten(self.foreground_subtle, self.background)
    }
}

/// `over`, kept as an `Hsla` for the callers that pack it themselves.
fn flatten(top: Hsla, bottom: Hsla) -> Hsla {
    let packed = over(top, bottom);
    gpui::Rgba {
        r: ((packed >> 16) & 0xff) as f32 / 255.0,
        g: ((packed >> 8) & 0xff) as f32 / 255.0,
        b: (packed & 0xff) as f32 / 255.0,
        a: 1.0,
    }
    .into()
}

/// `FlowTheme` takes packed `0x00RRGGBB`; daruda's palette is `Hsla`. The
/// conversion lives here rather than in the palette so the palette keeps
/// one colour type.
pub(super) fn rgb_u32(c: Hsla) -> u32 {
    let rgba = gpui::Rgba::from(c);
    let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    (to_byte(rgba.r) << 16) | (to_byte(rgba.g) << 8) | to_byte(rgba.b)
}

pub(super) struct FlowNodeRenderer {
    pub(super) palette: CardPalette,
}

#[cfg(test)]
thread_local! {
    /// How many cards have been drawn. The canvas is vendored and cannot be
    /// counted directly, but nothing draws a card except a canvas render, so
    /// this is the same signal one layer down. Thread-local for the same
    /// reason as `WORKSPACE_RENDERS`.
    pub(in crate::workspace) static CARDS_DRAWN: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// How much of a card fits in the box the canvas will draw it in.
///
/// Thresholds are screen widths rather than zoom levels: the question is
/// whether the content fits the box, and that stays the same question if the
/// card's declared size ever changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CardDensity {
    Full,
    Compact,
    Marker,
}

pub(super) fn density_for(screen_width: f32) -> CardDensity {
    if screen_width >= palette::FLOW_GRAPH_DENSITY_FULL_W {
        CardDensity::Full
    } else if screen_width >= palette::FLOW_GRAPH_DENSITY_COMPACT_W {
        CardDensity::Compact
    } else {
        CardDensity::Marker
    }
}

/// Kind chip and status badge, at the size where both words fit.
fn full_header(card: &CardData, accent: gpui::Rgba, p: CardPalette) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(palette::FLOW_GRAPH_CARD_ROW_GAP))
        .child(
            div()
                .px(px(palette::FLOW_GRAPH_CHIP_PAD_X))
                .rounded(px(palette::FLOW_GRAPH_CHIP_RADIUS))
                .bg(rgb(p.chip_bg))
                .text_color(rgb(p.text_mute))
                .text_size(px(palette::FLOW_GRAPH_CHIP_FONT_SIZE))
                .child(card.chip.clone()),
        )
        .child(div().flex_grow())
        .child(
            div()
                .text_color(accent)
                .text_size(px(palette::FLOW_GRAPH_CHIP_FONT_SIZE))
                .child(card.badge.clone()),
        )
}

fn full_body(card: &CardData, p: CardPalette) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(palette::FLOW_GRAPH_CARD_ROW_GAP))
        .child(
            div()
                .text_color(rgb(p.text_body))
                .text_size(px(palette::FLOW_GRAPH_ID_FONT_SIZE))
                .child(card.id.clone()),
        )
        .child(
            div()
                .text_color(rgb(p.text_mute))
                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                .child(card.meta.clone()),
        )
        .child(
            div()
                .text_color(rgb(p.text_subtle))
                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                .child(card.summary.clone()),
        )
}

/// The chip reduced to a mark: at this size its word would not fit.
///
/// Shape carries the kind, not colour — a round mark is an agent, a square one
/// a gate. The card's border is already spending colour on run status, and
/// DESIGN.md's rule against colour-only signals applies twice over when the
/// mark is six pixels wide.
fn kind_dot(card: &CardData, p: CardPalette) -> impl IntoElement {
    let mark = div()
        .flex_none()
        .w(px(palette::FLOW_GRAPH_KIND_DOT))
        .h(px(palette::FLOW_GRAPH_KIND_DOT))
        .bg(rgb(p.text_mute));
    match card.kind {
        CardKind::Gate => mark.rounded(px(palette::FLOW_GRAPH_KIND_DOT / 4.0)),
        CardKind::Agent => mark.rounded_full(),
    }
}

impl NodeRenderer for FlowNodeRenderer {
    fn render(&self, node: &Node, ctx: &mut RenderContext) -> AnyElement {
        #[cfg(test)]
        CARDS_DRAWN.with(|n| n.set(n.get() + 1));
        let Ok(card) = serde_json::from_value::<CardData>(node.data_ref().clone()) else {
            // A node the view did not stamp. Draw the shell so the graph
            // keeps its shape instead of losing a box.
            return ctx
                .node_card_shell_custom(node)
                .rounded(px(palette::FLOW_GRAPH_CARD_RADIUS))
                .bg(rgb(self.palette.card_bg))
                .border_1()
                .border_color(rgb(self.palette.pending_border))
                .into_any();
        };
        let accent = rgb(rgb_u32(card.accent.hsla()));
        let shell = ctx
            .node_card_shell_custom(node)
            .rounded(px(palette::FLOW_GRAPH_CARD_RADIUS))
            .bg(rgb(self.palette.card_bg))
            .border_1()
            .border_color(accent);

        // The box the canvas is about to draw this in, not the box the graph
        // declares: it scales with zoom and the text inside does not.
        match density_for(palette::FLOW_GRAPH_NODE_W * ctx.zoom()) {
            CardDensity::Full => shell
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .size_full()
                        .gap(px(palette::FLOW_GRAPH_CARD_ROW_GAP))
                        .p(px(palette::FLOW_GRAPH_CARD_PAD))
                        .child(full_header(&card, accent, self.palette))
                        .child(full_body(&card, self.palette)),
                )
                .into_any(),
            CardDensity::Compact => shell
                .child(
                    div()
                        .flex()
                        .items_center()
                        .size_full()
                        .gap(px(palette::FLOW_GRAPH_CARD_ROW_GAP))
                        .px(px(palette::FLOW_GRAPH_CHIP_PAD_X))
                        .child(kind_dot(&card, self.palette))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(self.palette.text_body))
                                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                                .child(card.id),
                        )
                        .child(
                            div()
                                .flex_shrink()
                                .truncate()
                                .text_color(accent)
                                .text_size(px(palette::FLOW_GRAPH_MARKER_FONT_SIZE))
                                .child(card.badge),
                        ),
                )
                .into_any(),
            // The id and nothing else. Which node this is stays the last thing
            // to go, because a wall of anonymous blocks answers no question a
            // person opened the graph to ask.
            CardDensity::Marker => shell
                .child(
                    div()
                        .flex()
                        .items_center()
                        .size_full()
                        .px(px(palette::FLOW_GRAPH_CHIP_PAD_X))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(self.palette.text_body))
                                .text_size(px(palette::FLOW_GRAPH_MARKER_FONT_SIZE))
                                .child(card.id),
                        ),
                )
                .into_any(),
        }
    }

    fn port_render(&self, node: &Node, port: &Port, ctx: &mut RenderContext) -> Option<AnyElement> {
        let frame = ctx.port_screen_frame(node, port)?;
        Some(
            frame
                .anchor_div()
                .rounded_full()
                .bg(rgb(rgb_u32(palette::FLOW_GRAPH_EDGE)))
                .into_any(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::main_area::flow_graph_pane::model::{
        AgentAxes, FailPolicy, GraphNode, GraphNodeKind, PromptSummary,
    };
    use std::time::Duration;

    fn agent_node(fail: FailPolicy) -> GraphNode {
        GraphNode {
            id: "review".into(),
            kind: GraphNodeKind::Agent {
                agent: AgentAxes {
                    id: "claude".into(),
                    model: None,
                    effort: Some("high".into()),
                    mode: None,
                },
                prompt: PromptSummary::Inline("review it".into()),
                output: "review.md".into(),
            },
            timeout: Duration::from_secs(600),
            cwd: None,
            fail,
        }
    }

    /// The card's one badge line answers "what happens next". Under a run
    /// that is the run's report; with no run it is the node's own policy.
    #[test]
    fn every_run_state_picks_its_own_accent() {
        let node = agent_node(FailPolicy::Halt);
        for (state, want) in [
            (NodeRunState::Pending, CardAccent::Pending),
            (NodeRunState::Running { attempt: 1 }, CardAccent::Running),
            (NodeRunState::Running { attempt: 2 }, CardAccent::Retried),
            (NodeRunState::Passed, CardAccent::Passed),
            (NodeRunState::Failed, CardAccent::Failed),
            (NodeRunState::Fixing, CardAccent::Retried),
        ] {
            assert_eq!(card_for(&node, state).accent, want, "for {state:?}");
        }
    }

    /// A first attempt is not worth saying — every node has one, and saying
    /// it on every card buries the card that says "3".
    #[test]
    fn a_first_attempt_reads_as_running_not_as_a_number() {
        let node = agent_node(FailPolicy::Halt);
        let first = card_for(&node, NodeRunState::Running { attempt: 1 }).badge;
        let third = card_for(&node, NodeRunState::Running { attempt: 3 }).badge;
        assert_ne!(first, third);
        assert!(
            third.contains('3'),
            "a later attempt names its number: {third}"
        );
    }

    /// With no run driving it, a node that repairs still says so — the empty
    /// `rerun` case draws no edge, so the card is the only place it shows.
    #[test]
    fn a_pending_node_reports_its_failure_policy() {
        let halt = card_for(&agent_node(FailPolicy::Halt), NodeRunState::Pending);
        assert!(halt.badge.is_empty(), "halting is the absence of a policy");

        let retry = card_for(
            &agent_node(FailPolicy::Retry { max_attempts: 2 }),
            NodeRunState::Pending,
        );
        assert!(retry.badge.contains('2'), "{}", retry.badge);
    }

    #[test]
    fn an_agent_card_lists_the_axes_worth_seeing() {
        let card = card_for(&agent_node(FailPolicy::Halt), NodeRunState::Pending);
        assert!(card.meta.contains("claude"), "{}", card.meta);
        assert!(card.meta.contains("high"), "{}", card.meta);
        assert!(
            card.meta.contains("10m"),
            "the timeout, in the file's own vocabulary: {}",
            card.meta
        );
        assert_eq!(card.summary, "review it");
    }

    /// A card at 1:1 says everything; the same card in a graph framed down to
    /// fit says only what still fits. The thresholds are the point — a card
    /// that kept all four rows at a third of its width would clip them.
    #[test]
    fn a_card_drops_rows_as_its_box_shrinks() {
        let w = palette::FLOW_GRAPH_NODE_W;
        assert_eq!(density_for(w * 1.0), CardDensity::Full);
        assert_eq!(density_for(w * 0.8), CardDensity::Full);
        // 0.6 → 150px: no room for the axes line, still room for the id.
        assert_eq!(density_for(w * 0.6), CardDensity::Compact);
        // 0.35 → 87px, which is where a six-node chain lands in a pane.
        assert_eq!(density_for(w * 0.35), CardDensity::Marker);
        assert_eq!(density_for(0.0), CardDensity::Marker);
    }
}
