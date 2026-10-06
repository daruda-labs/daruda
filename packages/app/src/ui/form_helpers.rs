//! Shared form-row helpers for settings and modals.
//!
//! Free functions are enough while each row has one fixed layout. Promote to a
//! builder only if a real second/third axis appears.

use crate::ui::theme;
use gpui::{IntoElement, SharedString, div, prelude::*, px};

const LABEL_WIDTH: f32 = 120.0;

/// Fixed-label form row; label colour flows through `gpui_component::Label`.
pub fn field_row(label: impl Into<SharedString>, input: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_LG))
        .child(
            div()
                .w(px(LABEL_WIDTH))
                .text_size(px(theme::FONT_SIZE_MD))
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
        .gap(px(theme::PAD_LG))
        .child(
            div()
                .text_size(px(theme::FONT_SIZE_MD))
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
        .gap(px(theme::GAP_LG))
        .child(div().w(px(LABEL_WIDTH)))
        .child(widget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render, TestAppContext, VisualTestContext, Window};

    struct FormProbe;

    impl Render for FormProbe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let field =
                |id: &'static str| div().id(id).debug_selector(move || id.into()).size(px(24.));
            div()
                .flex()
                .flex_col()
                .w(px(400.))
                .child(field_row("Label", field("row-input")))
                .child(checkbox_row(field("checkbox-input")))
                .child(field_column("Stacked label", field("column-input")))
        }
    }

    #[gpui::test]
    fn fields_and_checkboxes_share_a_gutter_but_stacked_fields_do_not(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        let window = cx.add_window(|_, _| FormProbe);
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let row = vcx.debug_bounds("row-input").unwrap();
        let checkbox = vcx.debug_bounds("checkbox-input").unwrap();
        let column = vcx.debug_bounds("column-input").unwrap();
        assert_eq!(row.origin.x, checkbox.origin.x);
        assert_eq!(
            row.origin.x - column.origin.x,
            px(LABEL_WIDTH + theme::GAP_LG)
        );
    }
}
