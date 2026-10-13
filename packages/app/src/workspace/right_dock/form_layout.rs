//! Geometry shared only by the Skills and Tools authoring forms.

use crate::ui::ButtonVariants as _;
use crate::ui::theme;
use gpui::{
    App, Context, Div, Entity, IntoElement, SharedString, Subscription, Window, div, prelude::*, px,
};

pub(super) fn field(label: impl IntoElement, control: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(px(theme::FORM_MODAL_FIELD_GAP))
        .child(label)
        .child(control)
}

pub(super) fn footer(cx: &App) -> gpui::Stateful<Div> {
    div()
        .id("library-form-footer")
        .debug_selector(|| "library-form-footer".into())
        .w_full()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_end()
        .gap(px(theme::GAP_LG))
        .border_t_1()
        .border_color(theme::current(cx).border)
        .pt(px(theme::FORM_MODAL_FOOTER_PADDING))
}

pub(super) fn source(text: impl Into<SharedString>, cx: &App) -> Div {
    let t = theme::current(cx);
    div()
        .min_w_0()
        .w_full()
        .bg(t.modal_input_bg)
        .border_y_1()
        .border_color(t.border)
        .p(px(theme::FORM_MODAL_FOOTER_PADDING))
        .text_size(px(theme::FONT_SIZE_MD))
        .text_color(t.text_body)
        .child(text.into())
}

pub(super) fn pair(left: impl IntoElement, right: impl IntoElement, window: &Window) -> Div {
    div()
        .flex()
        .when(
            window.viewport_size().width < px(theme::FORM_MODAL_STACK_AT),
            |row| row.flex_col(),
        )
        .gap(px(theme::FORM_MODAL_FIELD_SPACING))
        .child(div().flex_1().min_w_0().child(left))
        .child(div().flex_1().min_w_0().child(right))
}

pub(super) struct Choice<V> {
    state: Entity<crate::ui::select::SelectState>,
    values: Vec<V>,
    _subscription: Subscription,
}

impl<V: Copy + PartialEq + 'static> Choice<V> {
    pub fn new<E: 'static>(
        options: Vec<(V, String)>,
        selected: V,
        window: &mut Window,
        cx: &mut Context<E>,
        on_select: impl Fn(&mut E, V, &mut Context<E>) + 'static,
    ) -> Self {
        use crate::ui::select::{self, SelectEvent, SelectOption};
        let values: Vec<V> = options.iter().map(|(value, _)| *value).collect();
        let initial = values
            .iter()
            .position(|value| *value == selected)
            .map(|index| SharedString::from(index.to_string()));
        let options = options
            .into_iter()
            .enumerate()
            .map(|(index, (_, label))| SelectOption::new(index.to_string(), label))
            .collect();
        let state = cx.new(|cx| select::state_with_options(options, initial.as_ref(), window, cx));
        let choices = values.clone();
        let subscription = cx.subscribe(&state, move |this, _, event, cx| {
            if let SelectEvent::Confirm(Some(value)) = event
                && let Ok(index) = value.parse::<usize>()
                && let Some(value) = choices.get(index)
            {
                on_select(this, *value, cx);
            }
        });
        Self {
            state,
            values,
            _subscription: subscription,
        }
    }

    pub fn set(&mut self, value: V, window: &mut Window, cx: &mut App) {
        if let Some(index) = self.values.iter().position(|item| *item == value) {
            let value = SharedString::from(index.to_string());
            self.state
                .update(cx, |state, cx| state.set_selected_value(&value, window, cx));
        }
    }

    #[cfg(test)]
    pub fn selected(&self, cx: &App) -> Option<V> {
        self.state
            .read(cx)
            .selected_value()
            .and_then(|value| value.parse::<usize>().ok())
            .and_then(|index| self.values.get(index).copied())
    }

    pub fn render(&self, disabled: bool, tab: isize, cx: &App) -> gpui::AnyElement {
        crate::ui::select::select(&self.state, cx, tab)
            .w_full()
            .disabled(disabled)
            .into_any_element()
    }
}

pub(super) fn advanced(id: &'static str, open: bool) -> crate::ui::Button {
    crate::ui::button_with_icon(
        id,
        crate::surface::strings::common::field_advanced(),
        if open {
            crate::ui::icons::EXPAND_MORE
        } else {
            crate::ui::icons::CHEVRON_RIGHT
        },
    )
    .ghost()
    .justify_start()
}

pub(super) fn back(cx: &App) -> crate::ui::Button {
    crate::ui::button_icon("paste-back", crate::ui::icons::BACK, cx)
        .tooltip(crate::surface::strings::common::btn_back())
}

pub(super) fn actions(buttons: impl IntoElement, error: Option<&SharedString>) -> Div {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_LG))
        .children(error.map(|message| crate::ui::alert::error("form-save-error", message.clone())))
        .child(buttons)
}
