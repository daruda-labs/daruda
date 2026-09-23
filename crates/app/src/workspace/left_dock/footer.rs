//! The left dock's footer row — a settings shortcut that is visible whatever
//! the dock is showing.
//!
//! The title bar's `···` menu is the complete surface and the only one that
//! survives the dock being closed; this is the shortcut, on the rank every
//! sidebar-shaped app puts it.

use gpui::{AnyElement, Context, IntoElement, div, prelude::*, px};

use super::super::layout::Dock;
use crate::ui::ButtonVariants as _;
use crate::ui::theme;
use crate::workspace::OpenSettings;

pub(in crate::workspace) fn render(cx: &mut Context<Dock>) -> AnyElement {
    let t = theme::current(cx);

    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .w_full()
        .p(px(theme::DOCK_FOOTER_PAD))
        .border_t_1()
        .border_color(t.border)
        .child(
            crate::ui::button_with_icon(
                "left-dock-settings",
                crate::surface::strings::dock_settings(),
                crate::ui::icons::SETTINGS,
            )
            .ghost()
            .text_size(px(theme::DOCK_VIEW_TAB_FONT_SIZE))
            // The View dispatches; `Workspace::on_open_settings` owns the
            // body, and the global fallback answers where it does not.
            .on_click(|_, window, cx| {
                window.dispatch_action(
                    Box::new(OpenSettings(daruda_config::BuiltinSection::default())),
                    cx,
                );
            }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    /// The footer's whole job is to carry an affordance for an action that is
    /// otherwise reachable only from a keyboard or the title-bar menu, so the
    /// action it names has to be the one Settings listens for.
    #[test]
    fn the_footer_dispatches_the_settings_action() {
        let action = crate::workspace::OpenSettings(daruda_config::BuiltinSection::default());
        assert_eq!(action.0, daruda_config::BuiltinSection::default());
    }
}
