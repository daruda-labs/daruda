//! Shared form-row helpers for settings and modals.
//!
//! Free functions are enough while each row has one fixed layout. Promote to a
//! builder only if a real second/third axis appears.

use crate::ui::theme;
use gpui::{IntoElement, SharedString, div, prelude::*, px};

/// Fixed-label form row; label colour flows through `gpui_component::Label`.
pub fn field_row(label: impl Into<SharedString>, input: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::MODAL_FOOTER_GAP))
        .child(
            div()
                .w(px(theme::SETTINGS_LABEL_W))
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .child(crate::ui::Label::new(label)),
        )
        .child(div().flex_1().child(input))
}

/// Label above field, for columns too narrow for [`field_row`]'s label gutter
/// — the node inspector is 280px wide, where a side-by-side label leaves the
/// input unusable.
pub fn field_column(label: impl Into<SharedString>, body: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::MODAL_PANEL_GAP))
        .child(
            div()
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .child(crate::ui::Label::new(label)),
        )
        .child(div().w_full().child(body))
}

/// Checkbox row that aligns under [`field_row`] — empty
/// label-width gutter on the left, widget on the right.
pub fn checkbox_row(widget: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::MODAL_FOOTER_GAP))
        .child(div().w(px(theme::SETTINGS_LABEL_W)))
        .child(widget)
}
