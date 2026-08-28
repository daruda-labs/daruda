//! Shared layout helpers for Activity Bar option popovers.

use gpui::{Div, Pixels, Window, div, prelude::*, px};

use crate::ui::theme;

/// The popover shell: the design size on both axes, each capped against the
/// window so the panel can never outgrow the frame it opens in.
///
/// Width is capped for the same reason height is, and only that reason: the
/// fold editor is 430px and the window can be narrower than that leaves room
/// for. It does **not** keep the panel off the docks beside its pane — a
/// popover in a narrow pane overlaps its neighbours the way a menu does.
pub(super) fn panel_root(design_w: f32, window: &Window) -> Div {
    div()
        .w(panel_w(design_w, window))
        .max_h(panel_max_h(window))
        .overflow_hidden()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_LG))
}

fn panel_w(design_w: f32, window: &Window) -> Pixels {
    px(cap_to_viewport(
        design_w,
        f32::from(window.viewport_size().width),
    ))
}

fn panel_max_h(window: &Window) -> Pixels {
    px(cap_to_viewport(
        theme::AGENT_CHAT_RULES_PANEL_MAX_H,
        f32::from(window.viewport_size().height),
    ))
}

/// The design size, lowered when the window is too small to hold it with room
/// left over for the popover's own margin.
fn cap_to_viewport(design: f32, viewport: f32) -> f32 {
    design.min(viewport * theme::AGENT_CHAT_PANEL_VIEWPORT_FRACTION)
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
    use super::cap_to_viewport;
    use crate::ui::theme;

    /// Every design size the two axes actually pass in.
    const DESIGNS: [f32; 3] = [
        theme::AGENT_CHAT_RULES_PANEL_MAX_H,
        theme::AGENT_CHAT_RULES_PANEL_W,
        theme::AGENT_CHAT_OPTIONS_PANEL_W,
    ];

    #[test]
    fn the_viewport_fraction_only_ever_lowers_the_design_cap() {
        let fraction = theme::AGENT_CHAT_PANEL_VIEWPORT_FRACTION;
        assert!(
            (0.0..=1.0).contains(&fraction),
            "a fraction of the viewport"
        );
        for design in DESIGNS {
            for viewport in [320.0_f32, 480.0, 600.0, 800.0, 1600.0, 3840.0] {
                let resolved = cap_to_viewport(design, viewport);
                assert!(resolved <= design, "{viewport}px window grew the panel");
                assert!(
                    resolved < viewport,
                    "{viewport}px window left no room for the popover's own margin"
                );
            }
        }
    }

    #[test]
    fn a_window_narrower_than_the_fold_editor_shrinks_it() {
        // The cap only bites below ~538px of window width (430 / 0.8), which is
        // a hand-resized window rather than any capture size.
        let capped = cap_to_viewport(theme::AGENT_CHAT_RULES_PANEL_W, 480.0);
        assert!(
            capped < theme::AGENT_CHAT_RULES_PANEL_W,
            "the design width has to give way to a small window"
        );
    }
}
