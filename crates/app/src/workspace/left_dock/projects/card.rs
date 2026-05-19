//! Group / ungrouped-shell card chrome for the Worktrees view.
//!
//! Card is purely visual — radius + border + bg + padding. All inner
//! interaction (drag/drop, context menu, chevron toggle) stays on the
//! header / body elements the caller injects.

use crate::ui::theme;
use crate::workspace::layout::Dock;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, prelude::*, px};

/// Wrap a group header + its member projects in a Premium Card surface.
/// The card itself is non-interactive aside from the `hover` background
/// lift — every click target stays in `header` / `body`.
pub(in crate::workspace::left_dock::projects) fn group_card(
    header: AnyElement,
    body: AnyElement,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    let t = theme::current(cx);
    let hover_bg = t.worktree_card_hover_bg;
    div()
        .flex()
        .flex_col()
        .mx(px(theme::WORKTREE_CARD_MARGIN_X))
        .px(px(theme::WORKTREE_CARD_PAD_X))
        .py(px(theme::WORKTREE_CARD_PAD_Y))
        .rounded(px(theme::WORKTREE_CARD_RADIUS))
        .bg(t.worktree_card_bg)
        .border(px(theme::WORKTREE_CARD_BORDER_W))
        .border_color(t.worktree_card_border)
        .hover(move |d| d.bg(hover_bg))
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
        .mx(px(theme::WORKTREE_CARD_MARGIN_X))
        .px(px(theme::WORKTREE_CARD_PAD_X))
        .py(px(theme::WORKTREE_CARD_PAD_Y))
        .rounded(px(theme::WORKTREE_CARD_RADIUS))
        .child(body)
}
