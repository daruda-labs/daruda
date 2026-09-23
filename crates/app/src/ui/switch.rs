//! Shadow-free switch using the button's established keyboard/focus contract.

use gpui::{App, ElementId, ParentElement as _, Styled as _, div, prelude::FluentBuilder, px};
use gpui_component::button::ButtonCustomVariant;

use super::{Button, ButtonVariants as _, button_bare, theme};

/// Controlled switch: the caller commits the change before updating `checked`.
pub fn switch(id: impl Into<ElementId>, checked: bool, cx: &App) -> Button {
    let t = theme::current(cx);
    let track = div()
        .flex()
        .items_center()
        .when(checked, |el| el.justify_end())
        .w(px(theme::SETTINGS_SWITCH_W))
        .h(px(theme::SETTINGS_SWITCH_H))
        .px(px(theme::SETTINGS_SWITCH_INSET))
        .rounded_full()
        .bg(if checked {
            theme::ACCENT
        } else {
            t.text_subtle
        })
        .child(
            div()
                .size(px(theme::SETTINGS_SWITCH_THUMB))
                .rounded_full()
                .bg(theme::ACCENT_FG),
        );
    button_bare(id)
        .tab_stop(true)
        .custom(
            ButtonCustomVariant::new(cx)
                .foreground(t.text_primary)
                .hover(t.button_widget_bg_hover)
                .active(t.button_widget_bg_hover),
        )
        .w(px(theme::SETTINGS_SWITCH_TARGET_W))
        .h(px(theme::BUTTON_WIDGET_HEIGHT))
        .p(px(0.))
        .child(track)
}

#[cfg(test)]
mod tests;
