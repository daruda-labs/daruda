//! Shared list-page chrome; callers own scope, filtering and state transitions.

use gpui::{
    AnyElement, App, Div, ElementId, Entity, FontWeight, IntoElement, SharedString, Window, div,
    prelude::*, px,
};

use super::{Button, ButtonVariants as _, InputState, button_bare, disclosure, theme};

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
        .flex()
        .w_full()
        .min_w_0()
        .child(crate::input::search_with_clear(
            clear_id,
            clear_tooltip,
            state,
            has_query,
            on_clear,
            cx,
        ))
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
        .child(crate::empty_message(message, cx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Focusable as _, Render, TestAppContext, VisualTestContext};

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
            .w(px(240.))
            .debug_selector(|| "search-field-probe".into())
        }
    }

    #[gpui::test]
    fn search_clear_dispatches_once_and_disappears_after_the_owner_updates(
        cx: &mut TestAppContext,
    ) {
        crate::test_support::init_gpui_component(cx);
        let mut probe = None;
        let window = cx.add_window(|window, cx| {
            let entity = cx.new(|cx| Probe {
                input: cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_value(
                        "A long query whose caret must stay clear of the close button",
                        window,
                        cx,
                    );
                    input
                }),
                cleared: 0,
            });
            probe = Some(entity.clone());
            gpui_component::Root::new(entity, window, cx)
        });
        let probe = probe.unwrap();
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        let clear = vcx.debug_bounds("clear-query").unwrap();
        let field = vcx.debug_bounds("search-field-probe").unwrap();
        assert_eq!(field.size.height, px(28.));
        assert_eq!(clear.center().y, field.center().y);
        window
            .update(&mut vcx, |_, _, cx| {
                let probe = probe.read(cx);
                let input = probe.input.read(cx);
                let mut text = input.last_bounds().unwrap();
                text.origin -= input.scroll_handle().offset();
                assert!(
                    text.right() <= clear.left(),
                    "search text overlaps the clear action: {text:?}, {clear:?}"
                );
            })
            .unwrap();
        vcx.simulate_click(clear.center(), Default::default());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("clear-query").is_none());
        assert_eq!(probe.read_with(&vcx, |probe, _| probe.cleared), 1);
        assert_eq!(vcx.debug_bounds("search-field-probe").unwrap(), field);
        window
            .update(&mut vcx, |_, window, cx| {
                assert!(
                    probe
                        .read(cx)
                        .input
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
            })
            .unwrap();
    }
}
