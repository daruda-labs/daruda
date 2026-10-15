//! Transcript range boundaries and response outcome marks.

use super::*;

/// The tail window's boundary row: a rule with the label inset, rather than a
/// [`FoldRow`]. Deliberately a second shape in this module, not a third
/// `FoldRow` constructor — the row is not a content fold. Nothing collapses
/// *into* it; it marks the edge of the range the pane is showing, and its
/// content is the steps the window covers.
///
/// The label is centred between two equal rules in **both** states, so the row
/// does not move as it opens. What marks the flip is the chevron, which takes
/// [`DisclosureAxis::Vertical`] to stay distinct from the `▶`/`▼` tree glyph
/// every fold-row bar owns, plus the label itself naming the state it goes to.
pub(in crate::render) fn window_boundary_row(
    base: impl Into<ElementId>,
    toggle: impl Into<FoldToggle>,
    expanded: bool,
    label: SharedString,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let base = base.into();
    let rule_color = theme::dim_toward_gray(theme::agent_chat_border_tint(cx), dim);
    let rule = move || div().flex_1().border_t_1().border_color(rule_color);
    let chevron: Disclosure = disclosure((base.clone(), "chevron"), expanded)
        .axis(DisclosureAxis::Vertical)
        .color(theme::dim_toward_gray(theme::agent_chat_fg_subtle(cx), dim));
    let toggle = toggle.into().into_fn();
    let label = div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(label)
        .child(chevron);
    div()
        .id((base, "row"))
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_BOUNDARY_GAP))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _ev, window, cx| toggle(this, window, cx)))
        .child(rule())
        .child(label)
        .child(rule())
        .into_any_element()
}

/// The row marking where a Stop cut a run. Shares the boundary row's shape —
/// label centred between two rules — because both say the same kind of thing:
/// this is an edge in the transcript, not a message in it. It differs in
/// carrying no chevron and no click: nothing is folded away here, so there is
/// nothing to open. Lives beside [`window_boundary_row`] so the two cannot
/// drift apart on rule colour or label tone.
pub(in crate::render) fn interrupted_row(
    label: String,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let rule_color = theme::dim_toward_gray(theme::agent_chat_border_tint(cx), dim);
    let rule = move || div().flex_1().border_t_1().border_color(rule_color);
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_BOUNDARY_GAP))
        .child(rule())
        .child(
            div()
                .flex_none()
                .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(label)),
        )
        .child(rule())
        .into_any_element()
}

/// The standard trailing-slot mark summarizing a run's outcome. Every header
/// that represents a whole response carries exactly one — the response bar, the
/// tool-group bar, and a top-level assistant block (which *is* the whole response
/// when the run is trivial enough that no bar is emitted). Blocks nested under a
/// bar stay mark-free so a collapsed turn shows one verdict, not two.
///
/// The mark itself comes from [`status_icon`](crate::render::status_icon), the table the
/// plan region reads too, so a running run and a running step cannot disagree.
pub(in crate::render) fn rollup_glyph(
    rollup: Rollup,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &gpui::App,
) -> AnyElement {
    super::super::status_icon::status_icon(rollup, t, dim, cx)
}
