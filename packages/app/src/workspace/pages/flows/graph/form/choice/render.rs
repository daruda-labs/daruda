//! Picker geometry and the explicit custom-value fallback.

use super::{CUSTOM, Choice};
use crate::ui::{form_input, select::form_select, theme};
use gpui::{Context, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px};

impl Render for Choice {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let custom = self
            .select
            .read(cx)
            .selected_value()
            .is_some_and(|value| value.as_ref() == CUSTOM);
        div()
            .flex()
            .flex_col()
            .gap(px(theme::GAP_SM))
            .child(form_select(&self.select, cx, self.tab))
            .children(custom.then(|| form_input(&self.input, cx, self.tab + 1)))
    }
}
