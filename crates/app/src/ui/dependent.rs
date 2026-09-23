//! Container for rows that only take effect while a parent switch is on.

use gpui::{App, Div, Styled as _, div, prelude::FluentBuilder as _, px};

use super::theme;

/// Indent `children` under their parent row, and dim them while
/// `parent_on` is false. They stay editable: a value can be set before the
/// parent is switched on.
pub fn dependent(parent_on: bool, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .ml(px(theme::SETTINGS_DEPENDENT_INDENT))
        .border_l_1()
        .border_color(theme::current(cx).border)
        .when(!parent_on, |el| {
            el.opacity(theme::SETTINGS_DEPENDENT_OFF_OPACITY)
        })
}

#[cfg(test)]
mod tests {
    use super::theme;

    #[test]
    fn an_inactive_child_is_dimmed_but_still_visible() {
        const {
            assert!(theme::SETTINGS_DEPENDENT_OFF_OPACITY > 0.0);
            assert!(theme::SETTINGS_DEPENDENT_OFF_OPACITY < 1.0);
        }
    }
}
