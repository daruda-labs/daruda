//! The shared disclosure hit area and hover treatment.

use super::*;

/// Where the fold-toggle click lives on a [`FoldRow`] header: the whole row, or
/// the chevron glyph alone. Selected through [`FoldRow::toggle_on_chevron`], so it
/// stays private to the assembly.
#[derive(Clone, Copy)]
pub(super) enum ToggleTarget {
    /// The whole header row toggles the fold — generous hit area (section bars,
    /// text blocks).
    Row,
    /// Only the chevron toggles; the rest of the header stays inert so a header
    /// carrying selectable content (the tool-card title) doesn't fight text
    /// selection.
    Chevron,
}

/// The disclosure scaffold shared by every fold header: a full-width borderless
/// row that applies `toggle` and leads with the chevron. `target` picks where the
/// click lives.
pub(super) fn disclosure_row(
    base: ElementId,
    toggle: FoldToggle,
    expanded: bool,
    target: ToggleTarget,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> Stateful<Div> {
    // One base id yields both the row's click target and the chevron glyph's
    // identity — distinct yet stable across renders.
    let chevron: Disclosure = disclosure((base.clone(), "chevron"), expanded)
        .color(theme::dim_toward_gray(theme::agent_chat_fg_subtle(cx), dim));
    // Hover answers wherever the click lives, so the two are read off the same
    // `target` — a row-wide wash where the row toggles, a chip behind the glyph
    // where only the glyph does. Lighting the row for a chevron-only header
    // would promise a hit area that is not there.
    let hover = theme::dim_toward_gray(theme::agent_chat_hover_tint(cx), dim);
    // A text line's worth of height even when the header has no text. Most
    // headers carry a label and already clear it, so this reaches only the ones
    // that are a bare chevron — the conclusion of a restored turn, whose facts
    // were never measured in this process. Without it that row stands at the
    // glyph's own height and reads as a seam rather than as a row.
    let row = div()
        .id((base.clone(), "row"))
        .w_full()
        .min_w_0()
        .min_h(px(
            theme::agent_chat_font_size(cx) * theme::agent_chat_line_height(cx)
        ))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP));
    let toggle = toggle.into_fn();
    match target {
        ToggleTarget::Row => row
            .cursor_pointer()
            .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
            .hover(|s| s.bg(hover))
            .on_click(cx.listener(move |this, _ev, window, cx| toggle(this, window, cx)))
            .child(chevron),
        // Bind the click to the chevron itself; the row carries no click
        // handler, so selectable header content stays freely selectable.
        ToggleTarget::Chevron => {
            row.child(
                div()
                    .id((base, "chevron-hit"))
                    .flex_none()
                    .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
                    .hover(|s| s.bg(hover))
                    .child(chevron.on_toggle(
                        cx.listener(move |this, _ev, window, cx| toggle(this, window, cx)),
                    )),
            )
        }
    }
}
