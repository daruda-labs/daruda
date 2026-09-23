//! Shadow-free switch using the button's established keyboard/focus contract.

use gpui::{App, ElementId, ParentElement as _, Styled as _, div, prelude::FluentBuilder, px};
use gpui_component::button::ButtonCustomVariant;

use super::{Button, ButtonVariants as _, button_bare, theme};

/// Track, thumb and hit-target sizes for one switch tier.
struct Metrics {
    w: f32,
    h: f32,
    thumb: f32,
    inset: f32,
    target_w: f32,
    target_h: f32,
}

const SETTINGS: Metrics = Metrics {
    w: theme::SETTINGS_SWITCH_W,
    h: theme::SETTINGS_SWITCH_H,
    thumb: theme::SETTINGS_SWITCH_THUMB,
    inset: theme::SETTINGS_SWITCH_INSET,
    target_w: theme::SETTINGS_SWITCH_TARGET_W,
    target_h: theme::BUTTON_WIDGET_HEIGHT,
};

const COMPACT: Metrics = Metrics {
    w: theme::COMPACT_SWITCH_W,
    h: theme::COMPACT_SWITCH_H,
    thumb: theme::COMPACT_SWITCH_THUMB,
    inset: theme::COMPACT_SWITCH_INSET,
    target_w: theme::COMPACT_SWITCH_TARGET_W,
    target_h: theme::CONTROL_TARGET_SIZE,
};

/// Controlled switch: the caller commits the change before updating `checked`.
pub fn switch(id: impl Into<ElementId>, checked: bool, cx: &App) -> Button {
    sized(id, checked, &SETTINGS, cx)
}

/// The same control at list-row scale.
pub fn switch_compact(id: impl Into<ElementId>, checked: bool, cx: &App) -> Button {
    sized(id, checked, &COMPACT, cx)
}

fn sized(id: impl Into<ElementId>, checked: bool, m: &Metrics, cx: &App) -> Button {
    let t = theme::current(cx);
    let track = div()
        .flex()
        .items_center()
        .when(checked, |el| el.justify_end())
        .w(px(m.w))
        .h(px(m.h))
        .px(px(m.inset))
        .rounded_full()
        .bg(if checked {
            theme::ACCENT
        } else {
            t.text_subtle
        })
        .child(div().size(px(m.thumb)).rounded_full().bg(theme::ACCENT_FG));
    button_bare(id)
        .tab_stop(true)
        .custom(
            ButtonCustomVariant::new(cx)
                .foreground(t.text_primary)
                .hover(t.button_widget_bg_hover)
                .active(t.button_widget_bg_hover),
        )
        .w(px(m.target_w))
        .h(px(m.target_h))
        .p(px(0.))
        .child(track)
}

#[cfg(test)]
mod tests;
