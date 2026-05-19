//! Helpers that open `gpui_component::Dialog` based modals.
//!
//! Four shapes:
//!
//! - [`open_single_field_dialog`] — embeds `gpui_component::Input`.
//!   Cancel / OK buttons are provided by Dialog itself; Enter inside
//!   the input propagates to Dialog's `Confirm` action so OK fires
//!   without a click. On OK the trimmed text (or `None`) is forwarded
//!   to the caller.
//! - [`open_form_modal`] — daruda's modal entity is the entire body
//!   (form fields + validation banner + custom footer buttons). Dialog
//!   provides only outer chrome (panel bg, border, padding, backdrop,
//!   Escape-to-close). The entity dismisses itself by calling
//!   [`gpui_component::WindowExt::close_dialog`].
//! - [`open_confirm_dialog`] — title + body text + OK/Cancel footer.
//!   Caller supplies the OK handler. Used for short-lived destructive
//!   confirmations (Delete macro / skill / tool / worktree). Dialog
//!   owns OK / Cancel button rendering and dismissal.
//! - [`open_error_report_dialog`] — Layer 2 of the error-reporting
//!   pipeline. Mounts an [`ErrorReportModal`] for a captured
//!   [`ErrorReport`]; modal owns [Copy report] / [Open log file] /
//!   [Close] action buttons.

use std::rc::Rc;

use crate::ui::theme;
use daruda_store::observability::error_report::ErrorReport;
use gpui::{
    App, AppContext as _, ClickEvent, Context, Focusable, ParentElement as _, Pixels, SharedString,
    WeakEntity, Window, px,
};

use crate::surface::strings as s;
use crate::ui::WindowExt as _;
use crate::ui::dialog::{ButtonVariant, DialogButtonProps};
use crate::ui::{InputState, input};
use crate::workspace::Workspace;
use crate::workspace::error::modal::ErrorReportModal;
use crate::workspace::modal_view::ModalView;

/// Open a Dialog with one labelled `gpui_component::Input` and a
/// Cancel / OK footer. On OK the callback receives the trimmed text
/// (or `None` if blank) along with `&mut Workspace`.
///
/// Enter inside the single-line input propagates out via
/// `InputState::enter -> cx.propagate()` (see `input/state.rs` Enter
/// handler) so Dialog's `Confirm` action triggers `on_ok` without a
/// click. Escape is handled by Dialog's `Cancel` action.
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
    let placeholder: SharedString = placeholder.into();
    let initial_owned: Option<SharedString> = initial.map(|v| v.to_string().into());

    let state = cx.new(|cx_state| {
        let mut s = InputState::new(window, cx_state).placeholder(placeholder);
        if let Some(v) = initial_owned.clone() {
            s = s.default_value(v);
        }
        s
    });

    let title: SharedString = title.into();
    let on_submit = Rc::new(on_submit);
    let state_for_focus = state.clone();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let state_for_body = state.clone();
        let state_for_ok = state.clone();
        let workspace = workspace.clone();
        let on_submit = on_submit.clone();
        dialog
            .title(title.clone())
            .child(input(&state_for_body, cx, 0))
            .confirm()
            .button_props(DialogButtonProps::default().ok_text("OK"))
            .on_ok(move |_, window, app_cx| {
                let text = state_for_ok.read(app_cx).value().to_string();
                let trimmed = text.trim();
                let value = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
                if let Some(ws) = workspace.upgrade() {
                    ws.update(app_cx, |ws, cx| (on_submit)(ws, value, window, cx));
                }
                true
            })
    });

    // `Root::open_dialog` synchronously focuses the dialog's own
    // freshly-minted `FocusHandle` (see vendored
    // `gpui_component::root::open_dialog`). To land focus inside the
    // input instead, schedule our focus claim on a later update cycle
    // — by that time the dialog tree has been painted, the input's
    // `track_focus` has registered the handle in the focus map, and
    // our `window.focus(...)` wins. Doing this *after* `open_dialog`
    // (not before) ensures we run *after* root's focus assignment.
    //
    // `let _ = cx.update_window(...)` is intentional: if the window
    // is closed (or the dialog programmatically dismissed by a
    // synchronous validation callback fired in the same tick that
    // opened it) before the defer runs, the update returns `Err` and
    // the focus call becomes a harmless no-op. There is no current
    // caller that fast-dismisses inside the same tick; if one is
    // added, it should drop the dialog body itself rather than
    // relying on the deferred focus silently noop'ing.
    let handle = state_for_focus.read(cx).focus_handle(cx);
    let wh = window.window_handle();
    cx.defer(move |cx| {
        // SILENT-OK: focus restore on possibly-dismissed dialog
        let _ = cx.update_window(wh, |_, window, cx| window.focus(&handle, cx));
    });
}

/// Open a Dialog with a title, a single body element (typically the
/// confirmation prompt as plain text), and an OK / Cancel footer.
///
/// `ok_label` and `ok_variant` shape the destructive vs. neutral OK
/// button. `on_ok` runs inside the live `App` context after the user
/// confirms; the helper closes the dialog regardless of the handler's
/// outcome.
pub(in crate::workspace) fn open_confirm_dialog<F>(
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    ok_label: impl Into<SharedString>,
    ok_variant: ButtonVariant,
    on_ok: F,
    window: &mut Window,
    cx: &mut App,
) where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let title: SharedString = title.into();
    let body: SharedString = body.into();
    let ok_label: SharedString = ok_label.into();
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let on_ok = on_ok.clone();
        dialog
            .title(title.clone())
            .child(body.clone())
            .confirm()
            .button_props(
                DialogButtonProps::default()
                    .ok_text(ok_label.clone())
                    .ok_variant(ok_variant),
            )
            .on_ok(move |ev, window, app_cx| {
                (on_ok)(ev, window, app_cx);
                true
            })
    });
}

/// Open an OK-only alert dialog. No cancel, no destructive action —
/// just a title, a body, and a single dismiss button. Used by
/// [`Workspace::open_task_error_dialog`] (R-26 View error) to surface
/// the full `TaskState::Error.message` text, which the row truncates
/// to `RIGHT_PANEL_TASK_ERROR_TRUNCATE` chars.
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
/// backdrop, Escape-to-close). `close_button(false)` and
/// `overlay_closable(false)` suppress Dialog's own close / overlay
/// dismiss; the entity controls when to dismiss via
/// [`gpui_component::WindowExt::close_dialog`].
///
/// `width` is forwarded to `Dialog::width(...)` when supplied — pass
/// `None` to keep the Dialog default (~480px).
///
/// Replaces `Workspace::open_modal::<XxxModal, _>(...)` for Phase 4.c/d.
pub(in crate::workspace) fn open_form_modal<E, B>(
    title: impl Into<SharedString>,
    width: Option<Pixels>,
    build: B,
    window: &mut Window,
    cx: &mut App,
) where
    E: ModalView + 'static,
    B: FnOnce(&mut Window, &mut Context<E>) -> E + 'static,
{
    let title: SharedString = title.into();
    let entity = cx.new(|cx_modal| build(window, cx_modal));
    let entity_for_focus = entity.clone();

    window.open_dialog(cx, move |dialog, _window, _cx| {
        let mut d = dialog
            .child(entity.clone())
            .close_button(false)
            .overlay_closable(false);
        if !title.is_empty() {
            d = d.title(title.clone());
        }
        if let Some(w) = width {
            d = d.width(w);
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
    let title: SharedString = format!("{}{}", s::ERROR_MODAL_TITLE_PREFIX, report.title).into();
    open_form_modal(
        title,
        Some(px(theme::ERROR_MODAL_WIDTH)),
        move |window, cx| ErrorReportModal::new(report, window, cx),
        window,
        cx,
    );
}
