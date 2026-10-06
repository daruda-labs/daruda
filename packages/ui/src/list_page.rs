//! Shared list-page chrome; callers own scope, filtering and state transitions.

use gpui::{
    AnyElement, App, Div, ElementId, Entity, FontWeight, IntoElement, MouseButton, SharedString,
    Window, div, prelude::*, px,
};

use super::{
    Button, ButtonVariants as _, InputState, button_bare, button_icon, disclosure, icons, input,
    theme,
};

pub fn header(
    title: impl Into<SharedString>,
    scope: impl IntoElement,
    action: impl IntoElement,
    close: impl IntoElement,
    cx: &App,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(theme::GAP_STANDARD))
        .px(px(theme::DOCK_PAGE_PAD))
        .py(px(theme::PAD_STANDARD))
        .child(
            div()
                .flex_none()
                .text_size(px(theme::FONT_SIZE_MD))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::current(cx).text_primary)
                .child(title.into()),
        )
        .child(scope)
        .child(div().flex_1())
        .child(action)
        .child(close)
}

pub fn toolbar() -> Div {
    div().flex().items_center().gap(px(theme::GAP_STANDARD))
}

pub fn search(
    clear_id: &'static str,
    clear_tooltip: SharedString,
    state: &Entity<InputState>,
    has_query: bool,
    on_clear: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    div()
        .relative()
        .flex()
        .w_full()
        .min_w_0()
        .child(input(state, cx, ()))
        .when(has_query, |row| {
            row.child(
                button_icon(clear_id, icons::CLOSE, cx)
                    .debug_selector(move || clear_id.into())
                    .tooltip(clear_tooltip)
                    .absolute()
                    .right(px(theme::PAD_XS))
                    .top_0()
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        cx.stop_propagation();
                        on_clear(window, cx);
                    }),
            )
        })
}

pub fn results(message: impl Into<SharedString>, clear: Option<AnyElement>, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .flex_wrap()
        .gap(px(theme::GAP_SM))
        .text_size(px(theme::FONT_SIZE_MD))
        .text_color(theme::current(cx).text_muted)
        .child(message.into())
        .children(clear)
}

pub fn group_header(
    id: impl Into<ElementId>,
    disclosure_id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    expanded: bool,
    cx: &App,
) -> Button {
    button_bare(id)
        .ghost()
        .tab_stop(true)
        .w_full()
        .justify_start()
        .text_color(theme::current(cx).text_muted)
        .child(disclosure(disclosure_id, expanded))
        .child(label.into())
}

pub fn empty(message: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(theme::GAP_STANDARD))
        .py(px(theme::DOCK_PAGE_PAD))
        .text_size(px(theme::FONT_SIZE_MD))
        .text_color(theme::current(cx).text_muted)
        .child(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render, TestAppContext, VisualTestContext};

    struct Probe {
        input: Entity<InputState>,
        cleared: usize,
    }

    impl Render for Probe {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let probe = cx.entity().downgrade();
            search(
                "clear-query",
                "Clear".into(),
                &self.input,
                !self.input.read(cx).value().is_empty(),
                move |window, cx| {
                    probe
                        .update(cx, |probe, cx| {
                            probe.cleared += 1;
                            probe
                                .input
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            cx.notify();
                        })
                        .unwrap();
                },
                cx,
            )
        }
    }

    #[gpui::test]
    fn search_clear_dispatches_once_and_disappears_after_the_owner_updates(
        cx: &mut TestAppContext,
    ) {
        crate::test_support::init_gpui_component(cx);
        let window = cx.add_window(|window, cx| Probe {
            input: cx.new(|cx| {
                let mut input = InputState::new(window, cx);
                input.set_value("query", window, cx);
                input
            }),
            cleared: 0,
        });
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        let clear = vcx.debug_bounds("clear-query").unwrap();
        vcx.simulate_click(clear.center(), Default::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("clear-query").is_none());
        assert_eq!(window.read_with(&vcx, |probe, _| probe.cleared).unwrap(), 1);
    }
}
