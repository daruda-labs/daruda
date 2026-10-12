//! Retryable deletion: retain the target and diagnostic until success.

use crate::{
    surface::strings,
    ui::{WindowExt as _, button, button_danger, theme},
    workspace::ModalView,
};
use gpui::{
    App, ClickEvent, Context, FocusHandle, Focusable, IntoElement, Render, SharedString, Window,
    div, prelude::*, px,
};
use std::rc::Rc;

type DeleteAction = Rc<dyn Fn(&mut Window, &mut App) -> Result<(), SharedString>>;

struct DeleteModal {
    focus: FocusHandle,
    body: SharedString,
    error: Option<SharedString>,
    action: DeleteAction,
}

impl DeleteModal {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match (self.action)(window, cx) {
            Ok(()) => window.close_dialog(cx),
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }
}

impl Focusable for DeleteModal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl ModalView for DeleteModal {
    fn guards_close(&self) -> bool {
        true
    }
}

impl Render for DeleteModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(self.body.clone())
            .children(
                self.error
                    .as_ref()
                    .map(|error| crate::ui::alert::error("delete-error", error.clone())),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .child(
                        div()
                            .track_focus(&self.focus.clone().tab_stop(true).tab_index(0))
                            .border_1()
                            .rounded(px(theme::RADIUS_SM))
                            .border_color(theme::TRANSPARENT)
                            .when(self.focus.is_focused(window), |d| {
                                d.border_color(theme::PRIMARY)
                            })
                            .on_key_down(|event, window, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    cx.stop_propagation();
                                    window.close_dialog(cx);
                                }
                            })
                            .child(
                                button("delete-cancel", strings::common::btn_cancel())
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            ),
                    )
                    .child(
                        button_danger("delete-confirm", strings::common::btn_delete())
                            .tab_stop(true)
                            .tab_index(1)
                            .on_key_down(cx.listener(
                                |this, event: &gpui::KeyDownEvent, window, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        cx.stop_propagation();
                                        this.submit(window, cx);
                                    }
                                },
                            ))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.submit(window, cx);
                            })),
                    ),
            )
    }
}

pub(super) fn open(
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    action: impl Fn(&mut Window, &mut App) -> Result<(), SharedString> + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let body = body.into();
    let action = Rc::new(action);
    crate::workspace::dialog_helpers::open_form_modal(
        title,
        None,
        move |_, cx| DeleteModal {
            focus: cx.focus_handle(),
            body,
            action,
            error: None,
        },
        window,
        cx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};
    use std::cell::Cell;

    #[gpui::test]
    async fn modal_safety_delete_starts_on_cancel_and_explicit_confirmation_retries(
        cx: &mut TestAppContext,
    ) {
        let (window, _workspace) = crate::workspace::tests::build_workspace(cx);
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        let attempts = Rc::new(Cell::new(0));
        let counter = attempts.clone();
        vcx.update(|window, cx| {
            open(
                "Delete",
                "Confirm deletion",
                move |_, _| {
                    counter.set(counter.get() + 1);
                    if counter.get() == 1 {
                        Err("Try again".into())
                    } else {
                        Ok(())
                    }
                },
                window,
                cx,
            );
        });
        vcx.run_until_parked();
        vcx.simulate_keystrokes("tab enter");
        vcx.run_until_parked();
        assert_eq!(attempts.get(), 1);
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert_eq!(attempts.get(), 2);
        let counter = attempts.clone();
        vcx.update(|window, cx| {
            open(
                "Delete",
                "Confirm deletion",
                move |_, _| {
                    counter.set(counter.get() + 1);
                    Ok(())
                },
                window,
                cx,
            );
        });
        vcx.run_until_parked();
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert_eq!(attempts.get(), 2, "Enter on Cancel must not delete");
    }
}
