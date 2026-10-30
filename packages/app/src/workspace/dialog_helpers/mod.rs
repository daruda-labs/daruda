//! Helpers for opening `gpui_component::Dialog` based modals.
//!
//! Covers single-field dialogs, form-body modals, destructive confirmations,
//! and the error-report modal. Dialog owns outer chrome; guarded form entities
//! also own Escape handling alongside their custom body/footer behaviour.

mod field_error;

use std::rc::Rc;

use crate::ui::theme;
use daruda_store::observability::error_report::ErrorReport;
use gpui::{
    App, AppContext as _, Context, Entity, Focusable, ParentElement as _, Pixels, SharedString,
    WeakEntity, Window, div, prelude::*, px,
};

use crate::surface::strings as s;
use crate::ui::WindowExt as _;
use crate::ui::dialog::DialogButtonProps;
use crate::ui::{InputState, form_input};
use crate::workspace::Workspace;
use crate::workspace::error::modal::ErrorReportModal;
use crate::workspace::modal_view::ModalView;
use field_error::FieldError;

/// Open a one-input Dialog. Enter propagates to Dialog Confirm; OK receives
/// trimmed text or `None`.
pub(in crate::workspace) fn open_single_field_dialog<Cb>(
    workspace: WeakEntity<Workspace>,
    title: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    initial: Option<&str>,
    on_submit: Cb,
    window: &mut Window,
    cx: &mut App,
) where
    Cb: Fn(&mut Workspace, Option<String>, &mut Window, &mut Context<Workspace>) + 'static,
{
    open_validated_single_field_dialog(
        workspace,
        title,
        placeholder,
        initial,
        move |ws, value, window, cx| {
            on_submit(ws, value, window, cx);
            Ok(())
        },
        window,
        cx,
    );
}

/// [`open_single_field_dialog`] whose submit can refuse the value: `Err`
/// keeps the dialog open with the message under the field, so the user fixes
/// what they typed instead of retyping it after a toast. Editing the field
/// clears the message.
pub(in crate::workspace) fn open_validated_single_field_dialog<Cb>(
    workspace: WeakEntity<Workspace>,
    title: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    initial: Option<&str>,
    on_submit: Cb,
    window: &mut Window,
    cx: &mut App,
) where
    Cb: Fn(
            &mut Workspace,
            Option<String>,
            &mut Window,
            &mut Context<Workspace>,
        ) -> Result<(), SharedString>
        + 'static,
{
    let placeholder: SharedString = placeholder.into();
    let initial_owned: Option<SharedString> = initial.map(|v| v.to_string().into());

    let state = cx.new(|cx_state| {
        let mut s = InputState::new(window, cx_state).placeholder(placeholder);
        if let Some(v) = initial_owned.clone() {
            s = s.default_value(v);
        }
        s
    });

    let field_error = cx.new(|cx| FieldError::new(&state, cx));

    let title: SharedString = title.into();
    let on_submit = Rc::new(on_submit);
    let state_for_focus = state.clone();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let state_for_body = state.clone();
        let state_for_ok = state.clone();
        let error_for_ok = field_error.clone();
        let workspace = workspace.clone();
        let on_submit = on_submit.clone();
        dialog
            .title(title.clone())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("single-modal-field")
                            .debug_selector(|| "single-modal-field".into())
                            .child(form_input(&state_for_body, cx, 0)),
                    )
                    .child(field_error.clone()),
            )
            .confirm()
            .button_props(DialogButtonProps::default().ok_text(s::common::btn_ok()))
            .on_ok(move |_, window, app_cx| {
                let text = state_for_ok.read(app_cx).value().to_string();
                let trimmed = text.trim();
                let value = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
                let Some(ws) = workspace.upgrade() else {
                    return true;
                };
                match ws.update(app_cx, |ws, cx| (on_submit)(ws, value, window, cx)) {
                    Ok(()) => true,
                    Err(message) => {
                        error_for_ok.update(app_cx, |error, cx| error.show(message, cx));
                        false
                    }
                }
            })
    });

    // `Root::open_dialog` synchronously focuses the dialog's own
    // freshly-minted `FocusHandle`. Scheduling our claim on a later
    // update cycle lets it land after that assignment, once the
    // input's `track_focus` has registered its handle in the focus map.
    //
    // If the window closes (or the dialog is dismissed synchronously
    // in the same tick it opened in) before the defer runs,
    // `cx.update_window` returns `Err` and the focus call becomes a
    // harmless no-op.
    let handle = state_for_focus.read(cx).focus_handle(cx);
    let wh = window.window_handle();
    cx.defer(move |cx| {
        // SILENT-OK: focus restore on possibly-dismissed dialog
        let _ = cx.update_window(wh, |_, window, cx| window.focus(&handle, cx));
    });
}

/// Open an OK-only alert with a title, body, and dismiss button.
/// [`Workspace::open_task_error_dialog`] uses it to show the full task error.
pub(in crate::workspace) fn open_alert_dialog(
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    ok_label: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let title: SharedString = title.into();
    let body: SharedString = body.into();
    let ok_label: SharedString = ok_label.into();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .title(title.clone())
            .child(body.clone())
            .alert()
            .button_props(DialogButtonProps::default().ok_text(ok_label.clone()))
    });
}

/// Open a Dialog whose body is `entity` rendered in full — fields,
/// validation banner, and footer buttons all live on the entity.
///
/// Dialog supplies the outer chrome only (panel bg, border, padding,
/// backdrop, Escape-to-close). Guarded forms route overlay dismissal through
/// their close policy; unguarded forms retain their explicit-button policy.
/// The entity controls when to dismiss via
/// [`gpui_component::WindowExt::close_dialog`].
///
/// `width` is forwarded to `Dialog::width(...)` when supplied — pass
/// `None` to keep the Dialog default (~480px). The returned entity lives
/// while the dialog is open, so a weak handle to it says whether it still is.
pub(in crate::workspace) fn open_form_modal<E, B>(
    title: impl Into<SharedString>,
    width: Option<Pixels>,
    build: B,
    window: &mut Window,
    cx: &mut App,
) -> Entity<E>
where
    E: ModalView + 'static,
    B: FnOnce(&mut Window, &mut Context<E>) -> E + 'static,
{
    let title: SharedString = title.into();
    let entity = cx.new(|cx_modal| build(window, cx_modal));
    let entity_for_focus = entity.clone();

    window.open_dialog(cx, move |dialog, window, cx| {
        let guarded = entity.read(cx).guards_close();
        let close_entity = entity.clone();
        let body = div().child(entity.clone()).when(guarded, |body| {
            body.on_key_down(move |event, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    close_entity.update(cx, |modal, cx| modal.request_close(window, cx));
                }
            })
        });
        let overlay_entity = entity.clone();
        let mut d = dialog
            .keyboard(!guarded)
            .child(body)
            .close_button(guarded)
            .overlay_closable(guarded)
            .when(guarded, |dialog| {
                dialog.on_cancel(move |_, window, cx| {
                    overlay_entity.update(cx, |modal, cx| modal.request_close(window, cx));
                    false
                })
            });
        if entity.read(cx).fit_viewport() {
            let inset = crate::ui::dialog::window_paddings(window);
            let footer_entity = entity.clone();
            d = d
                .p(px(theme::FORM_MODAL_PADDING))
                .footer(move |_, _, _window, cx| {
                    let close_entity = footer_entity.clone();
                    vec![
                        div()
                            .w_full()
                            .on_key_down(move |event, window, cx| {
                                if guarded && event.keystroke.key == "escape" {
                                    cx.stop_propagation();
                                    close_entity
                                        .update(cx, |modal, cx| modal.request_close(window, cx));
                                }
                            })
                            .children(
                                footer_entity.update(cx, |modal, cx| modal.render_footer(cx)),
                            ),
                    ]
                })
                .margin_top(px(theme::PAD_XL))
                .max_h(
                    window.viewport_size().height
                        - inset.top
                        - inset.bottom
                        - px(theme::TITLE_BAR_HEIGHT + theme::PAD_XL + theme::PAD_XL),
                );
        }
        if !title.is_empty() {
            d = if entity.read(cx).fit_viewport() {
                d.title(
                    div()
                        .id("library-form-title")
                        .debug_selector(|| "library-form-title".into())
                        .w_full()
                        .text_size(px(theme::FORM_MODAL_TITLE_FONT_SIZE))
                        .pb(px(theme::FORM_MODAL_FOOTER_PADDING))
                        .border_b_1()
                        .border_color(theme::current(cx).border)
                        .child(title.clone()),
                )
            } else {
                d.title(title.clone())
            };
        }
        if let Some(w) = width {
            let available = window.viewport_size().width - px(theme::PAD_XL + theme::PAD_XL);
            d = d.width(w.min(available));
        }
        d
    });

    // `Root::open_dialog` synchronously focuses the dialog's own
    // `FocusHandle`. Schedule our claim *after* that so the modal's
    // primary input is the active focus once the dialog paints.
    let handle = entity_for_focus.read(cx).focus_handle(cx);
    let wh = window.window_handle();
    cx.defer(move |cx| {
        // SILENT-OK: focus restore on possibly-dismissed dialog
        let _ = cx.update_window(wh, |_, window, cx| window.focus(&handle, cx));
    });
    entity_for_focus
}

/// Open the Layer 2 Details modal for a captured [`ErrorReport`].
/// `report` is moved in — the dialog owns its own copy independent
/// of the live toast queue, so a concurrent auto-expire of the source
/// toast doesn't affect what the user sees in the modal.
pub(in crate::workspace) fn open_error_report_dialog(
    report: ErrorReport,
    window: &mut Window,
    cx: &mut App,
) {
    let title: SharedString = format!("{}{}", s::error_modal::title_prefix(), report.title).into();
    open_form_modal(
        title,
        Some(px(theme::ERROR_MODAL_WIDTH)),
        move |window, cx| ErrorReportModal::new(report, window, cx),
        window,
        cx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn group_creation_uses_authoring_field_geometry(cx: &mut gpui::TestAppContext) {
        let (window, workspace) = crate::workspace::tests::build_workspace(cx);
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.update(|window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.on_new_group(&crate::workspace::NewGroup, window, cx)
            });
        });
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        assert_eq!(
            vcx.debug_bounds("single-modal-field").unwrap().size.height,
            px(36.)
        );
    }

    #[gpui::test]
    fn a_refused_submit_keeps_the_dialog_open_until_the_value_passes(
        cx: &mut gpui::TestAppContext,
    ) {
        let (window, workspace) = crate::workspace::tests::build_workspace(cx);
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.update(|window, cx| {
            open_validated_single_field_dialog(
                workspace.downgrade(),
                "Rename",
                "name",
                Some("bad"),
                |_, value, _, _| match value.as_deref() {
                    Some("bad") => Err("refused".into()),
                    _ => Ok(()),
                },
                window,
                cx,
            );
        });
        vcx.run_until_parked();
        let dialog_up =
            |vcx: &mut gpui::VisualTestContext| vcx.update(crate::ui::WindowExt::has_active_dialog);
        let error_shown = |vcx: &mut gpui::VisualTestContext| {
            vcx.update(|window, _| window.refresh());
            vcx.run_until_parked();
            vcx.debug_bounds("single-modal-error").is_some()
        };

        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert!(dialog_up(&mut vcx), "a refused value keeps the dialog");
        assert!(error_shown(&mut vcx), "with the reason under the field");

        vcx.simulate_input("x");
        vcx.run_until_parked();
        assert!(!error_shown(&mut vcx), "editing clears the stale reason");

        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert!(!dialog_up(&mut vcx), "an accepted value closes it");
    }
}
