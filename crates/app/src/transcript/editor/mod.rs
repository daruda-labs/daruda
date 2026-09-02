//! The Fold and Filter editors, and the chrome they share.
//!
//! Both are host-neutral: they take the value, an id namespace, a type size and
//! a set of callbacks, and know nothing about who is showing them. The chat
//! pane opens them in its Activity Bar popover to change what one pane shows;
//! the Settings agent catalog opens the same two to author the defaults a pane
//! starts on. Neither can drift from the other, because there is only one.

use std::rc::Rc;

use gpui::{App, Div, IntoElement, Pixels, SharedString, Window, div, prelude::*, px};

use crate::ui::theme;

pub(crate) mod filter;
pub(crate) mod fold;
pub(crate) mod state;

/// The footer button that hands the axis back to what it departed from.
///
/// The wording is the same in both hosts and correct in both: a pane returns to
/// the agent's stated value, and an agent row returns to the built-in one.
/// `disabled` is the host's call — a value that merely *equals* the target can
/// still be an override worth undoing, so the editor never derives it.
pub(crate) struct ResetSpec {
    pub disabled: bool,
    pub on_reset: Rc<dyn Fn(&mut App)>,
}

/// The popover shell an editor sits in: the design size on both axes, each
/// capped against the window so the panel can never outgrow the frame it opens
/// in. Both hosts open the editors this way — the chat pane behind an Activity
/// Bar chip, Settings behind an agent row's field.
///
/// Width is capped for the same reason height is, and only that reason: the
/// fold editor is 430px and the window can be narrower than that leaves room
/// for. It does **not** keep the panel off whatever sits beside it — a popover
/// in a narrow frame overlaps its neighbours the way a menu does.
pub(crate) fn panel_root(design_w: f32, window: &Window) -> Div {
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

/// The band a panel scrolls: rules and facets outgrow the popover, headings and
/// footers must not move when they do.
pub(crate) fn scroll_region(id: impl Into<gpui::ElementId>) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
}

pub(crate) fn fixed_region() -> Div {
    div().flex_none().flex().flex_col().gap(px(theme::GAP_SM))
}

pub(crate) fn panel_heading(label: String, cx: &App) -> impl IntoElement {
    div()
        .text_color(theme::current(cx).text_subtle)
        .child(SharedString::from(label))
}

/// The footer both editors end with, or nothing in a host that offers no
/// return.
pub(crate) fn reset_footer(id: SharedString, label: String, reset: Option<ResetSpec>) -> Div {
    let Some(reset) = reset else {
        return div().flex_none();
    };
    use crate::ui::{ButtonVariants as _, Disableable as _, Sizable as _, button};
    fixed_region().child(
        button(id, label)
            .ghost()
            .xsmall()
            .disabled(reset.disabled)
            .on_click(move |_, _window, app| (reset.on_reset)(app)),
    )
}

#[cfg(test)]
mod tests {
    use super::cap_to_viewport;
    use crate::ui::theme;

    /// Every design size a host actually passes in.
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
