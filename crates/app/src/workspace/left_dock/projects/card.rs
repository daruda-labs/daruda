//! Group / ungrouped-shell card chrome for the Lanes view.
//!
//! Card is purely visual — radius + border + bg + padding. All inner
//! interaction (drag/drop, context menu, chevron toggle) stays on the
//! header / body elements the caller injects.

use crate::ui::theme;
use crate::workspace::layout::Dock;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, px};

/// Wrap a group header + its member projects in a Premium Card surface.
/// The card itself is non-interactive — every click target stays in
/// `header` / `body`, and the card carries no hover fill of its own so
/// the inner row hover (which sits on top of the card) stays visible.
///
/// `is_active` flips the surface to the "selected group" treatment when
/// the focused project lives inside this group.
pub(in crate::workspace::left_dock::projects) fn group_card(
    header: AnyElement,
    body: AnyElement,
    is_active: bool,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    let t = theme::current(cx);
    let bg = if is_active {
        t.lane_card_active_bg
    } else {
        t.lane_card_bg
    };
    div()
        .flex()
        .flex_col()
        .mx(px(theme::LANE_CARD_MARGIN_X))
        .px(px(theme::LANE_CARD_PAD_X))
        .py(px(theme::LANE_CARD_PAD_Y))
        .rounded(px(theme::LANE_CARD_RADIUS))
        .bg(bg)
        .border(px(theme::LANE_CARD_BORDER_W))
        .border_color(t.overlay_active)
        .child(header)
        .child(body)
}

/// "Shell" card used for ungrouped projects so they sit at the same
/// visual rank as group cards. No border / no bg — only padding +
/// rounded corners so hover affordances inside still have a clean bound.
pub(in crate::workspace::left_dock::projects) fn ungrouped_shell(
    body: AnyElement,
    _cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_col()
        .mx(px(theme::LANE_CARD_MARGIN_X))
        .px(px(theme::LANE_CARD_PAD_X))
        .py(px(theme::LANE_CARD_PAD_Y))
        .rounded(px(theme::LANE_CARD_RADIUS))
        .child(body)
}
