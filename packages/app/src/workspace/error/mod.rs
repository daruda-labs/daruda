//! Workspace-side error reporting entry point.
//!
//! Every surfaced error flows through [`Workspace::report_error`], which
//! routes it to two surfaces: the on-disk NDJSON log ([`LogWriter::global`])
//! and the live toast ([`ToastLayer`](super::toast_layer::ToastLayer)).

pub(in crate::workspace) mod modal;
pub(in crate::workspace) mod toast;

use daruda_store::observability::error_report::ErrorReport;
use daruda_store::observability::log_writer::LogWriter;
use gpui::Context;

use self::toast::ToastId;
use crate::workspace::Workspace;

impl Workspace {
    /// Surface an error to the user and the on-disk log. Safe to call
    /// from any `&mut Workspace` context; persistence and rendering
    /// updates fire from this single call.
    pub fn report_error(&mut self, report: ErrorReport, cx: &mut Context<Self>) {
        // 1. Persistence — best-effort. LogWriter may not be installed
        // (e.g. early-init tests) or may have been disabled if its
        // directory could not be created.
        if let Some(writer) = LogWriter::global() {
            writer.append(report.clone());
        }

        #[cfg(test)]
        self.error_history.insert(0, report.clone());

        // 2. Live toast — queue, expiry sweep, and render owned by
        // ToastLayer. Updating the child entity is re-entrant-safe in
        // GPUI (parent may update child freely). ToastLayer calls its
        // own cx.notify(); no Workspace repaint needed here.
        self.toast_layer.update(cx, |tl, cx| tl.push(report, cx));
    }

    /// Hand-dismiss the toast with the given stable id. Stale ids
    /// (e.g. the toast already auto-expired between the user's click
    /// and this handler) are a silent no-op. Routed from the toast
    /// widget's ✕ button.
    pub(in crate::workspace) fn dismiss_error_toast(
        &mut self,
        id: ToastId,
        cx: &mut Context<Self>,
    ) {
        self.toast_layer.update(cx, |tl, cx| tl.dismiss_id(id, cx));
    }

    /// Every report surfaced so far, newest-first.
    #[cfg(test)]
    pub(in crate::workspace) fn error_history(&self) -> &[ErrorReport] {
        &self.error_history
    }

    /// Read-only accessor for the live toast queue. Used by tests.
    #[cfg(test)]
    pub(in crate::workspace) fn error_toasts<'a>(
        &self,
        cx: &'a gpui::App,
    ) -> &'a self::toast::ErrorToastQueue {
        &self.toast_layer.read(cx).queue
    }
}
