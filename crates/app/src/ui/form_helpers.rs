//! Plain helper functions for form rows shared across daruda's
//! settings window and form modals.
//!
//! Pulled out of `settings_window::SettingsWindow::{field_row,
//! checkbox_row}` (originally `pub(super)`) so callers outside the
//! settings window — modal `Render` impls, future tool/task config
//! panels — can reuse the exact same row geometry instead of
//! re-rolling a `flex_row + label-width + input-flex` chain.
//!
//! These are free functions, not builders. Each has a single layout
//! and no optional knobs, so a `RenderOnce` builder would be
//! ceremony for no gain. If a third axis (label color, input width
//! cap, helper text below) appears, promote the helper to a builder
//! at that point.
//!
//! ```ignore
//! use crate::ui::form_helpers::{checkbox_row, field_row};
//!
//! div().child(field_row("Font", font_select))
//!      .child(field_row("Theme", theme_select))
//!      .child(checkbox_row(my_checkbox))
//! ```

use crate::ui::theme;
use gpui::{IntoElement, SharedString, div, prelude::*, px};

/// Form row with a fixed-width label on the left and a flex-grow
/// input on the right. Label width is `theme::SETTINGS_LABEL_W`.
/// Uses `gpui_component::Label` for the label cell so the text colour
/// flows from `cx.theme().foreground` (which `apply_daruda_palette`
/// retones on every light-mode switch). No `cx` argument needed.
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
