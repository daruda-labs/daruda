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
    w: 30.0,
    h: 18.0,
    thumb: 10.0,
    inset: 3.0,
    target_w: 38.0,
    target_h: 32.0,
};

const COMPACT: Metrics = Metrics {
    w: 24.0,
    h: 14.0,
    thumb: 10.0,
    inset: 2.0,
    target_w: 28.0,
    target_h: theme::CONTROL_TARGET_SIZE,
};

/// Sibling row actions reserve the same space as the compact control renders.
pub const COMPACT_TARGET_WIDTH: f32 = COMPACT.target_w;

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
