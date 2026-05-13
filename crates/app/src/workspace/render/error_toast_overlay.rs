//! Error-toast overlay — visual half of Layer 1 of the
//! error-reporting pipeline. The data half lives in
//! [`super::super::error_toast::ErrorToastQueue`].
//!
//! Renders a vertical stack of toast pills above the status bar.
//! Each toast carries:
//!
//! - severity tint bar (left edge),
//! - severity glyph (`ℹ` / `⚠` / `✕`),
//! - title (single-line) over message (single-line, dim),
//! - `[×N]` chip when `repeat_count >= 2`,
//! - `[Copy]` and `[Details]` action buttons,
//! - `[✕]` dismiss button.
//!
//! Snapshot pattern: the renderer never reads back into the
//! `Workspace` entity. The caller in `render/mod.rs` clones the
//! relevant `ErrorReport` fields into [`ToastSnapshot`] before
//! mounting this widget so the inner closures stay `'static`.

use crate::ui::theme;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{
    ClickEvent, ClipboardItem, IntoElement, RenderOnce, SharedString, WeakEntity, Window, div,
    prelude::*, px,
};

use crate::surface::strings as s;
use crate::ui::{button, button_header_action};
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers;
use crate::workspace::error_toast::ToastId;

/// Per-toast snapshot mounted into the overlay element. Cheap-clone
/// fields only — the renderer never touches the live `ErrorReport`
/// inside the queue.
#[derive(Clone)]
pub(in crate::workspace) struct ToastSnapshot {
    /// Stable id captured at snapshot time. The dismiss click handler
    /// uses this rather than a positional index so a concurrent
    /// auto-expire that shifts the queue can't redirect the click to
    /// a different toast.
    pub(in crate::workspace) id: ToastId,
    pub(in crate::workspace) title: SharedString,
    pub(in crate::workspace) message: SharedString,
    pub(in crate::workspace) repeat_count: u32,
    pub(in crate::workspace) severity: ErrorSeverity,
    /// Plain-text rendering captured at snapshot time. Pasted on
    /// `[Copy]` click — capturing it here means the click handler
    /// stays `'static` without reaching back into the workspace.
    pub(in crate::workspace) plain_text: SharedString,
    /// Full report captured at snapshot time. The `[Details]` click
    /// handler hands this to [`open_error_report_dialog`] so the modal
    /// stays consistent even if the source toast auto-expires before
    /// the user clicks.
    pub(in crate::workspace) report: ErrorReport,
}

/// Top-level overlay element. Renders nothing when `toasts` is empty.
#[derive(IntoElement)]
pub(in crate::workspace) struct ErrorToastOverlay {
    pub(in crate::workspace) toasts: Vec<ToastSnapshot>,
    pub(in crate::workspace) workspace: WeakEntity<Workspace>,
}

impl RenderOnce for ErrorToastOverlay {
    fn render(self, _window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let workspace = self.workspace;
        // Float the toast stack above the status bar without taking
        // part in the Workspace root's flex column. Using `absolute()`
        // here is what keeps the workspace content (`middle_row`)
        // from getting squeezed every time a toast appears.
        let mut stack = div()
            .absolute()
            .right(px(0.0))
            .bottom(px(theme::STATUS_BAR_HEIGHT))
            .flex()
            .flex_col()
            .items_end()
            .gap(px(theme::TOAST_STACK_GAP))
            .pb(px(theme::TOAST_STACK_BOTTOM_PAD))
            .pr(px(theme::STATUS_BAR_PAD_X));

        for snapshot in self.toasts {
            stack = stack.child(toast_pill(snapshot, workspace.clone(), cx));
        }

        stack
    }
}

fn toast_pill(
    snap: ToastSnapshot,
    workspace: WeakEntity<Workspace>,
    cx: &gpui::App,
) -> impl IntoElement {
    let t = theme::current(cx);
    let pill_bg = t.toast_bg;
    let pill_border = t.toast_border;
    let pill_text = t.toast_text;
    let pill_text_dim = t.toast_text_dim;
    let repeat_bg = t.toast_repeat_bg;
    let tint = severity_tint(snap.severity, t);
    let glyph = severity_glyph(snap.severity);
    let id = snap.id;

    // Row layout:
    // [tint-bar] [glyph] [title / message] [×N?] [Copy] [Details] [✕]
    div()
        .id(SharedString::from(format!("error-toast-{id}")))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::TOAST_GAP))
        .px(px(theme::TOAST_PAD_X))
        .py(px(theme::TOAST_PAD_Y))
        .min_w(px(theme::TOAST_MIN_W))
        .max_w(px(theme::TOAST_MAX_W))
        .bg(pill_bg)
        .border_1()
        .border_color(pill_border)
        .rounded(px(theme::TOAST_RADIUS))
        // Severity bar on the left edge — visual prefix to the glyph.
        .child(
            div()
                .w(px(theme::TOAST_SEVERITY_BAR_W))
                .h(px(theme::TOAST_FONT_SIZE * 2.0))
                .bg(tint)
                .rounded(px(theme::TOAST_SEVERITY_BAR_W / 2.0)),
        )
        .child(
            div()
                .text_size(px(theme::TOAST_TITLE_FONT_SIZE))
                .text_color(tint)
                .child(SharedString::from(glyph)),
        )
        // Title + message column. Takes whatever horizontal space the
        // pill has after the leading icons and trailing buttons.
        .child(
            div()
                .flex()
                .flex_col()
                .flex_grow()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_size(px(theme::TOAST_TITLE_FONT_SIZE))
                        .text_color(pill_text)
                        .overflow_hidden()
                        .child(snap.title.clone()),
                )
                .when(!snap.message.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(px(theme::TOAST_FONT_SIZE - 1.0))
                            .text_color(pill_text_dim)
                            .overflow_hidden()
                            .child(snap.message.clone()),
                    )
                }),
        )
        .when(snap.repeat_count >= 2, |el| {
            let label = format!("{}{}", s::TOAST_REPEAT_PREFIX, snap.repeat_count);
            el.child(
                div()
                    .px(px(theme::TOAST_REPEAT_PAD_X))
                    .py(px(theme::TOAST_REPEAT_PAD_Y))
                    .bg(repeat_bg)
                    .rounded(px(theme::TOAST_RADIUS / 2.0))
                    .text_size(px(theme::TOAST_REPEAT_FONT_SIZE))
                    .text_color(pill_text)
                    .child(SharedString::from(label)),
            )
        })
        .child(copy_button(id, snap.plain_text.clone()))
        .child(details_button(id, snap.report.clone()))
        .child(dismiss_button(id, workspace, cx))
}

fn copy_button(toast_id: ToastId, plain_text: SharedString) -> impl IntoElement {
    let element_id = format!("error-toast-{toast_id}-copy");
    button(SharedString::from(element_id), s::TOAST_BUTTON_COPY).on_click(
        move |_: &ClickEvent, _window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(plain_text.to_string()));
        },
    )
}

fn details_button(toast_id: ToastId, report: ErrorReport) -> impl IntoElement {
    let element_id = format!("error-toast-{toast_id}-details");
    // Capturing `report` and cloning on each fire keeps the listener
    // re-entrant — GPUI can dispatch the same `Fn` more than once if
    // the user rapidly re-opens / closes the dialog without touching
    // a different toast. ErrorReport's clone is cheap (small heap
    // strings + a BTreeMap of context entries).
    button(SharedString::from(element_id), s::TOAST_BUTTON_DETAILS).on_click(
        move |_: &ClickEvent, window, cx| {
            dialog_helpers::open_error_report_dialog(report.clone(), window, cx);
        },
    )
}

fn dismiss_button(
    toast_id: ToastId,
    workspace: WeakEntity<Workspace>,
    cx: &gpui::App,
) -> impl IntoElement {
    let element_id = format!("error-toast-{toast_id}-dismiss");
    button_header_action(SharedString::from(element_id), s::TOAST_BUTTON_DISMISS, cx).on_click(
        move |_: &ClickEvent, _window, app_cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(app_cx, |ws, cx| {
                    ws.dismiss_error_toast(toast_id, cx);
                });
            }
        },
    )
}

fn severity_tint(severity: ErrorSeverity, t: &crate::ui::theme::DarudaTheme) -> gpui::Hsla {
    match severity {
        ErrorSeverity::Info => t.toast_tint_info,
        ErrorSeverity::Warning => t.toast_tint_warning,
        ErrorSeverity::Error => t.toast_tint_error,
    }
}

const fn severity_glyph(severity: ErrorSeverity) -> &'static str {
    match severity {
        ErrorSeverity::Info => s::TOAST_ICON_INFO,
        ErrorSeverity::Warning => s::TOAST_ICON_WARNING,
        ErrorSeverity::Error => s::TOAST_ICON_ERROR,
    }
}
