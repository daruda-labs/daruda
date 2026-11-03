//! How the inputs form draws: one labelled control per input, and the
//! footer that runs or cancels.

use gpui::{
    ClickEvent, Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Window, div, prelude::*, px,
};

use super::{Control, Field, FlowInputsModal};
use crate::surface::strings as s;
use crate::ui::select;
use crate::ui::{WindowExt as _, button, button_primary, checkbox, form_input, theme};
use crate::workspace::ModalView;

impl FlowInputsModal {
    fn field(ix: usize, field: &Field, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::current(cx);
        let tab = ix as isize;
        let label: SharedString = if field.decl.required {
            s::flow::inputs_label_required(&field.decl.name).into()
        } else {
            field.decl.name.clone().into()
        };
        let control = match &field.control {
            Control::Text(state) => form_input(state, cx, tab).into_any_element(),
            Control::Prose(state) => form_input(state, cx, tab).into_any_element(),
            Control::Choice(state) => select::form_select(state, cx, tab).into_any_element(),
            // The description is what the box says yes to, so it sits beside
            // the box rather than above it.
            Control::Toggle(checked) => checkbox(
                SharedString::from(format!("flow-input-{}", field.decl.name)),
                field
                    .decl
                    .description
                    .clone()
                    .unwrap_or_else(|| field.decl.name.clone()),
                tab,
            )
            .checked(*checked)
            .on_click(
                cx.listener(move |this, checked: &bool, _, cx| this.set_toggle(ix, *checked, cx)),
            )
            .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_FIELD_GAP))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(t.text_body)
                    .child(label),
            )
            .children(
                (!matches!(field.control, Control::Toggle(_)))
                    .then(|| field.decl.description.clone())
                    .flatten()
                    .map(|description| {
                        div()
                            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                            .text_color(t.text_muted)
                            .child(description)
                    }),
            )
            .child(crate::ui::validated_field(
                SharedString::from(format!("flow-input-error-{}", field.decl.name)),
                control,
                field.error.as_ref(),
            ))
    }
}

impl ModalView for FlowInputsModal {
    fn fit_viewport(&self) -> bool {
        true
    }

    fn render_footer(&mut self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        vec![
            crate::ui::form_footer(cx)
                .child(
                    button("flow-inputs-cancel", s::common::btn_cancel()).on_click(
                        cx.listener(|_, _: &ClickEvent, window, cx| window.close_dialog(cx)),
                    ),
                )
                .child(
                    button_primary("flow-inputs-run", s::flow::inputs_run()).on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.submit(window, cx)),
                    ),
                )
                .into_any_element(),
        ]
    }
}

impl Render for FlowInputsModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fields: Vec<_> = self
            .fields
            .iter()
            .enumerate()
            .map(|(ix, field)| Self::field(ix, field, cx))
            .collect();
        div()
            .flex()
            .flex_col()
            .track_focus(&self.panel_focus_handle)
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .children(fields)
    }
}
