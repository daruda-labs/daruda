//! Expanded prompt layout uses the shared modal and authoring field geometry.
use super::Editor;
use crate::surface::strings as s;
use crate::ui::{WindowExt as _, button, button_primary, form_input, theme};
use gpui::{Context, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px};

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(theme::GAP_LG))
            .child(form_input(&self.input, cx, 0))
            .children(
                self.error
                    .as_ref()
                    .map(|error| crate::ui::alert::error("flow-prompt-conflict", error.clone())),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(theme::GAP_SM))
                    .child(
                        button("flow-prompt-cancel", s::common::btn_cancel())
                            .tab_stop(true)
                            .tab_index(1)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        button_primary("flow-prompt-apply", s::common::btn_ok())
                            .tab_stop(true)
                            .tab_index(2)
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                    ),
            )
    }
}
