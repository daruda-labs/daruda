use gpui::{Context, IntoElement, Render, Window, div, prelude::*, px};

use super::AliasModal;
use crate::surface::strings as s;
use crate::ui::{self, theme};

impl Render for AliasModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .tab_group()
            .child(
                div()
                    .text_size(px(theme::FONT_SIZE_SM))
                    .text_color(theme::current(cx).text_muted)
                    .mb(px(theme::PAD_STANDARD))
                    .child(s::project::alias_hint()),
            )
            .child(ui::form_input(&self.input, cx, 0))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
                    .child(
                        ui::button("project-alias-cancel", s::common::btn_cancel())
                            .tab_stop(true)
                            .tab_index(1)
                            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                    )
                    .child(
                        ui::button_primary("project-alias-save", s::common::btn_save())
                            .tab_stop(true)
                            .tab_index(2)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {}
