//! How a card is drawn — header, body, chips and markers at each density,
//! and the canvas `NodeRenderer` that picks between them.

use gpui::{
    AnyElement, Element as _, IntoElement, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px, rgb,
};

use crate::surface::strings as s;
use crate::ui::flow_canvas::{Node, NodeRenderer, Port, RenderContext};
use crate::ui::theme::palette;

#[cfg(test)]
use super::CARDS_DRAWN;
use super::{
    CardAccent, CardData, CardDensity, CardKind, CardPalette, FlowNodeRenderer, ICON_RETRIES,
    card_outline, density_for, rgb_u32,
};

/// Kind chip and status badge, at the size where both words fit.
fn full_header(card: &CardData, accent: gpui::Rgba, p: CardPalette) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .w_full()
        // A card is a fixed width and all four of these can be true at once —
        // a retrying node that breaks a rule and has just lost its pin. The row
        // clips, and the badge gives up its width first: it is the longest, and
        // it is the one whose full text the inspector carries anyway. Without
        // this it drew past the card's own border.
        .overflow_hidden()
        .gap(px(palette::FLOW_GRAPH_CARD_ROW_GAP))
        .child(
            div()
                .flex_none()
                .px(px(palette::FLOW_GRAPH_CHIP_PAD_X))
                .rounded(px(palette::FLOW_GRAPH_CHIP_RADIUS))
                .bg(rgb(p.chip_bg))
                .text_color(rgb(p.text_mute))
                .text_size(px(palette::FLOW_GRAPH_CHIP_FONT_SIZE))
                .child(card.chip.clone()),
        )
        .children(policy_chip(card, p))
        .children(issue_marker(card, p))
        .child(div().flex_grow())
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(rgb(badge_ink(card, accent, p)))
                .text_size(px(palette::FLOW_GRAPH_CHIP_FONT_SIZE))
                .child(card.badge.clone()),
        )
}

/// What colour the badge's words take.
///
/// The accent, except when it is `Pending` — that one is a hairline meant for
/// a border, and as text it is not there. A pending card's badge is not
/// nothing, though: it is why a pin has just gone, which is the news on that
/// card and was drawn in a colour nobody could read.
fn badge_ink(card: &CardData, accent: gpui::Rgba, p: CardPalette) -> u32 {
    match card.accent {
        CardAccent::Pending => p.text_body,
        _ => rgb_u32(gpui::Hsla::from(accent)),
    }
}

/// A cycle glyph and a count: this node tries again on failure, this many
/// times.
///
/// A glyph and not a word because a card is 250pt and the header can hold
/// four things at once — spelled out, all four truncated to nothing. Muted
/// like the kind chip: it is how the node is set up, not news.
fn policy_chip(card: &CardData, p: CardPalette) -> Option<impl IntoElement> {
    (!card.policy.is_empty()).then(|| {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(palette::FLOW_GRAPH_POLICY_GLYPH_GAP))
            .text_color(rgb(p.text_mute))
            .text_size(px(palette::FLOW_GRAPH_CHIP_FONT_SIZE))
            .child(
                gpui::svg()
                    .size(px(palette::FLOW_GRAPH_POLICY_GLYPH_SIZE))
                    .flex_none()
                    .path(ICON_RETRIES)
                    .text_color(rgb(p.text_mute)),
            )
            .child(card.policy.clone())
    })
}

/// How many rules this node breaks, when it breaks any.
///
/// Beside the kind chip and in its own colour, so it does not compete with the
/// status badge on the other end of the row: a card can be green from the last
/// run and broken since, and both have to be readable at once. The count is
/// the whole message — which rules is the inspector's to say, and the card has
/// no room for a sentence.
fn issue_marker(card: &CardData, p: CardPalette) -> Option<impl IntoElement> {
    (card.issues > 0).then(|| {
        div()
            .flex_none()
            .px(px(palette::FLOW_GRAPH_CHIP_PAD_X))
            .rounded(px(palette::FLOW_GRAPH_CHIP_RADIUS))
            .text_color(rgb(p.issue))
            .text_size(px(palette::FLOW_GRAPH_CHIP_FONT_SIZE))
            .child(s::flow::graph_issue_count(card.issues))
    })
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
            // One line, like the summary below and for the same reason: the
            // card is a fixed height, so a second line does not add a line —
            // it pushes the header out of the box.
            div()
                .min_w_0()
                .truncate()
                .text_color(rgb(p.text_mute))
                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                .child(card.meta.clone()),
        )
        .child(
            // One line: the card is a fixed height and this is a preview of a
            // prompt, so a second line pushes the header out of the box rather
            // than telling anyone more.
            div()
                .min_w_0()
                .truncate()
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
        let selected = ctx.graph.selected_node().contains(&node.id());
        let Ok(card) = serde_json::from_value::<CardData>(node.data_ref().clone()) else {
            // A node the view did not stamp. Draw the shell so the graph
            // keeps its shape instead of losing a box.
            let (line, thick) = card_outline(selected, self.palette.pending_border, self.palette);
            return ctx
                .node_card_shell_custom(node)
                .rounded(px(palette::FLOW_GRAPH_CARD_RADIUS))
                .bg(rgb(self.palette.card_bg))
                .when(thick, |s| s.border_2())
                .when(!thick, |s| s.border_1())
                .border_color(rgb(line))
                .into_any();
        };
        let accent = rgb(rgb_u32(card.accent.hsla()));
        let (line, thick) = card_outline(selected, rgb_u32(card.accent.hsla()), self.palette);
        let shell = ctx
            .node_card_shell_custom(node)
            .rounded(px(palette::FLOW_GRAPH_CARD_RADIUS))
            .bg(rgb(self.palette.card_bg))
            .when(thick, |s| s.border_2())
            .when(!thick, |s| s.border_1())
            .border_color(rgb(line));

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
