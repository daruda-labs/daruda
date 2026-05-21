//! Layer 2 of the error-reporting pipeline — the Details modal.
//!
//! Opened from a toast pill's `[Details]` button, this dialog renders
//! the full plain-text [`ErrorReport`] body (severity, location, message,
//! source chain, context, backtrace, system-info trailer) inside a
//! scrollable monospace panel and exposes three actions:
//!
//! - **Copy report** — writes the full plain-text rendering to the
//!   clipboard. The button label flips to `Copied` for a 1-second
//!   confirmation and reverts via a background timer.
//! - **Open log file** — defers to the system handler (`open` on
//!   macOS) so the user lands in their default text editor on
//!   today's NDJSON log file.
//! - **Close** — dismisses via [`gpui_component::WindowExt::close_dialog`].
//!
//! The modal owns its full body (G9): Dialog supplies only the outer
//! chrome (panel bg, border, padding, backdrop, ESC handling). The
//! report is captured at open time so dismissing doesn't depend on
//! the live toast queue (which may have already auto-expired the
//! source toast).

use std::time::Duration;

use crate::ui::theme;
use daruda_store::observability::error_report::ErrorReport;
use daruda_store::observability::log_writer;
use gpui::{
    App, ClickEvent, ClipboardItem, Context, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    Render, SharedString, Task, Window, div, prelude::*, px,
};

use crate::surface::strings as s;
use crate::ui::Disableable as _;
use crate::ui::WindowExt as _;
use crate::ui::button;
use crate::workspace::ModalView;

/// How long the `Copy report` button shows the `Copied` confirmation
/// before reverting. Short enough to feel responsive, long enough that
/// a glance catches the change.
const COPIED_LABEL_DURATION: Duration = Duration::from_secs(1);

pub(in crate::workspace) struct ErrorReportModal {
    /// Captured at open time so the modal stays consistent if the
    /// source toast auto-expires before dismissal. Currently read only
    /// by tests (`#[cfg(test)] fn report()`); kept for future
    /// severity-driven styling / inline source-location surfacing.
    #[allow(dead_code)]
    report: ErrorReport,
    /// Cached plain-text rendering. Computed once at construction so
    /// the same string flows into both the body element and the
    /// clipboard write — no risk of the two diverging after a future
    /// `to_plain_text` tweak.
    body_text: SharedString,
    focus_handle: FocusHandle,
    /// `true` while the post-copy `Copied` label is up. Driven by
    /// [`Self::_copied_revert_task`].
    copied: bool,
    /// Background timer that flips `copied` back to `false`. Replaced
    /// on every Copy click so a rapid second click resets the window.
    _copied_revert_task: Option<Task<()>>,
}

impl ErrorReportModal {
    pub(in crate::workspace) fn new(
        report: ErrorReport,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let body_text = report.to_plain_text().into();
        Self {
            report,
            body_text,
            focus_handle: cx.focus_handle(),
            copied: false,
            _copied_revert_task: None,
        }
    }

    fn dismiss(&self, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }

    fn copy_to_clipboard(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.body_text.to_string()));
        self.copied = true;
        cx.notify();

        self._copied_revert_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_LABEL_DURATION).await;
            // SILENT-OK: modal may drop before auto-dismiss timer fires
            let _ = this.update(cx, |this, cx| {
                if this.copied {
                    this.copied = false;
                    cx.notify();
                }
            });
        }));
    }

    fn open_log_file(&self, _window: &mut Window, cx: &mut App) {
        // The day's daily file is the primary surface; size-rolled
        // ordinals share the same directory and the user can browse
        // siblings from there. Falls back gracefully when the home
        // directory cannot be resolved.
        if let Some(dir) = log_writer::log_dir() {
            let date_path = log_writer::today_log_path()
                .filter(|p| p.exists())
                .unwrap_or(dir);
            cx.open_url(&format!("file://{}", date_path.display()));
        }
    }
}

impl Focusable for ErrorReportModal {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for ErrorReportModal {}

impl Render for ErrorReportModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        let body_bg = t.error_modal_body_bg;
        let body_border = t.error_modal_body_border;
        let body_text = t.toast_text;
        let body = div()
            .id("error-modal-body")
            .flex()
            .flex_col()
            .min_h(px(0.0))
            .max_h(px(theme::ERROR_MODAL_BODY_MAX_H))
            .overflow_y_scroll()
            .p(px(theme::ERROR_MODAL_BODY_PAD))
            .bg(body_bg)
            .border_1()
            .border_color(body_border)
            .rounded(px(theme::MODAL_PANEL_RADIUS / 2.0))
            .text_size(px(theme::ERROR_MODAL_BODY_FONT_SIZE))
            .text_color(body_text)
            .font_family(daruda_terminal::default_terminal_font().family.clone())
            .child(self.body_text.clone());

        let copy_label = if self.copied {
            s::error_modal_button_copied()
        } else {
            s::error_modal_button_copy()
        };

        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("error-modal-open-log", s::error_modal_button_open_log())
                    .disabled(log_writer::today_log_path().is_none())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_log_file(window, cx);
                    })),
            )
            .child(button("error-modal-copy", copy_label).on_click(cx.listener(
                |this, _: &ClickEvent, _window, cx| {
                    this.copy_to_clipboard(cx);
                },
            )))
            .child(
                button("error-modal-close", s::error_modal_button_close()).on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        this.dismiss(window, cx);
                    },
                )),
            );

        div()
            .flex()
            .flex_col()
            .key_context("ErrorReportModal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key.as_str() == "escape" {
                    this.dismiss(window, cx);
                    cx.stop_propagation();
                }
            }))
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body)
            .child(footer)
    }
}

#[cfg(test)]
#[allow(dead_code)] // exposed for tests that exercise the modal without rendering it.
impl ErrorReportModal {
    pub(in crate::workspace) fn report(&self) -> &ErrorReport {
        &self.report
    }

    pub(in crate::workspace) fn body_text(&self) -> &SharedString {
        &self.body_text
    }

    pub(in crate::workspace) fn copied(&self) -> bool {
        self.copied
    }

    /// Test-only entry into [`Self::copy_to_clipboard`] — the click
    /// handler that drives it lives inside a closure and isn't
    /// directly callable from tests.
    pub(in crate::workspace) fn copy_to_clipboard_for_test(&mut self, cx: &mut Context<Self>) {
        self.copy_to_clipboard(cx);
    }
}
