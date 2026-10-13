//! Layout and choices for the Skills and Tools authoring forms.

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
        crate::ui::select::form_select(&self.state, cx, tab)
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Render, TestAppContext, VisualTestContext};

    struct Probe {
        single: Entity<crate::ui::InputState>,
        multi: Entity<crate::ui::InputState>,
        choice: Choice<bool>,
    }

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(400.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .id("single")
                        .debug_selector(|| "single".into())
                        .child(crate::ui::form_input(&self.single, cx, 0)),
                )
                .child(
                    div()
                        .id("multi")
                        .debug_selector(|| "multi".into())
                        .child(crate::ui::form_input(&self.multi, cx, 1)),
                )
                .child(
                    div()
                        .id("choice")
                        .debug_selector(|| "choice".into())
                        .child(self.choice.render(false, 2, cx)),
                )
                .child(
                    div()
                        .id("readonly")
                        .debug_selector(|| "readonly".into())
                        .child(crate::ui::readonly_form_input("Project", cx)),
                )
        }
    }

    #[gpui::test]
    fn form_controls_share_height_and_text_insets(cx: &mut TestAppContext) {
        let _ = crate::workspace::tests::build_workspace(cx);
        let window = cx.add_window(|window, cx| Probe {
            single: cx.new(|cx| crate::ui::InputState::new(window, cx).default_value("Name")),
            multi: cx.new(|cx| {
                crate::ui::InputState::new(window, cx)
                    .auto_grow(2, 4)
                    .default_value("First\nSecond")
            }),
            choice: Choice::new(
                vec![(true, "Project".into())],
                true,
                window,
                cx,
                |_, _, _| {},
            ),
        });
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let single = vcx.debug_bounds("single").unwrap();
        let multi = vcx.debug_bounds("multi").unwrap();
        assert_eq!(single.size.height, px(36.));
        assert_eq!(
            vcx.debug_bounds("choice").unwrap().size.height,
            single.size.height
        );
        assert_eq!(
            vcx.debug_bounds("readonly").unwrap().size.height,
            single.size.height
        );
        window
            .update(&mut vcx, |probe, _, cx| {
                for (state, outer) in [(&probe.single, single), (&probe.multi, multi)] {
                    let text = state.read(cx).last_bounds().unwrap();
                    assert_eq!(text.left() - outer.left(), px(10.));
                    assert_eq!(text.top() - outer.top(), px(8.));
                    assert_eq!(outer.bottom() - text.bottom(), px(8.));
                }
            })
            .unwrap();
    }
}
