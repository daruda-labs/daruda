//! Shared layout helpers for Activity Bar option popovers.

use gpui::{Div, Pixels, Window, div, prelude::*, px};

use crate::ui::theme;

pub(super) fn panel_root(width: f32, max_h: Pixels) -> Div {
    div()
        .w(px(width))
        .max_h(max_h)
        .overflow_hidden()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_LG))
}

pub(super) fn panel_max_h(window: &Window) -> Pixels {
    px(theme::AGENT_CHAT_RULES_PANEL_MAX_H
        .min(f32::from(window.viewport_size().height) * theme::AGENT_CHAT_PANEL_VIEWPORT_FRACTION))
}

pub(super) fn scroll_region(id: impl Into<gpui::ElementId>) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
}

pub(super) fn fixed_region() -> Div {
    div().flex_none().flex().flex_col().gap(px(theme::GAP_SM))
}

#[cfg(test)]
mod tests {
    use crate::ui::theme;

    #[test]
    fn the_viewport_fraction_only_ever_lowers_the_design_cap() {
        let cap = theme::AGENT_CHAT_RULES_PANEL_MAX_H;
        let fraction = theme::AGENT_CHAT_PANEL_VIEWPORT_FRACTION;
        assert!(
            (0.0..=1.0).contains(&fraction),
            "a fraction of the viewport"
        );
        for viewport in [320.0_f32, 480.0, 600.0, 800.0, 1600.0, 3840.0] {
            let resolved = cap.min(viewport * fraction);
            assert!(resolved <= cap, "{viewport}px window grew the panel");
            assert!(
                resolved < viewport,
                "{viewport}px window left no room for the popover's own margin"
            );
        }
    }
}
