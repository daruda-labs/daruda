//! Workspace-side error reporting entry point.
//!
//! Every surfaced error (PTY thread death, MCP reload failure,
//! filesystem watcher init failure, …) flows through
//! [`Workspace::report_error`]. This module owns the routing logic so
//! the surfaces it touches stay in one place:
//!
//! 1. Append the report to the on-disk NDJSON log
//!    (`~/.daruda/logs/<profile>/daruda-YYYY-MM-DD.log`) via
//!    [`LogWriter::global`].
//! 2. Push it onto the in-memory `error_history` ring (capped at 50)
//!    so a future "Show recent errors" command palette entry can
//!    reach reports the user dismissed before they auto-expired.
//! 3. Delegate the live toast (queue, expiry sweep, render) to
//!    [`ToastLayer`](super::toast_layer::ToastLayer).

pub(in crate::workspace) mod modal;
pub(in crate::workspace) mod toast;

use daruda_store::observability::error_report::ErrorReport;
use daruda_store::observability::log_writer::LogWriter;
use gpui::Context;

use crate::workspace::Workspace;
use self::toast::ToastId;

/// Cap on the in-memory ring of recent reports. Tuned to fit comfortably
/// in a future "Show recent errors" command palette entry without
/// forcing scrollback.
const HISTORY_CAP: usize = 50;

impl Workspace {
    /// Surface an error to the user and the on-disk log. Safe to call
    /// from any `&mut Workspace` context; persistence and rendering
    /// updates fire from this single call.
    ///
    /// Replaces ad-hoc `eprintln!("daruda: …")` call sites — those
    /// only show up if the user launched daruda from a terminal with
    /// stderr attached, which is rarely the case for a `.app` bundle.
    pub fn report_error(&mut self, report: ErrorReport, cx: &mut Context<Self>) {
        // 1. Persistence — best-effort. LogWriter may not be installed
        // (e.g. early-init tests) or may have been disabled if its
        // directory could not be created.
        if let Some(writer) = LogWriter::global() {
            writer.append(report.clone());
        }

        // 2. Long-tail history (50 most recent, newest first).
        self.error_history.insert(0, report.clone());
        if self.error_history.len() > HISTORY_CAP {
            self.error_history.truncate(HISTORY_CAP);
        }

        // 3. Live toast — queue, expiry sweep, and render owned by
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

    /// Read-only accessor for the in-memory history ring. Used by
    /// tests and (future) command-palette surfaces.
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
