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
//! 3. Push it onto the live [`ErrorToastQueue`](super::error_toast::ErrorToastQueue)
//!    that the renderer mirrors above the status bar — dedup,
//!    capacity-3 FIFO, severity-driven auto-dismiss.
//! 4. Schedule a 1 Hz background timer that drives the queue's
//!    expiry sweeps. The timer is rescheduled (not duplicated) on
//!    every push.

use std::time::Duration;

use daruda_store::observability::error_report::ErrorReport;
use daruda_store::observability::log_writer::LogWriter;
use gpui::Context;

use super::Workspace;
use super::error_toast::{ErrorToastQueue, ToastId};

/// Cap on the in-memory ring of recent reports. Tuned to fit comfortably
/// in a future "Show recent errors" command palette entry without
/// forcing scrollback.
const HISTORY_CAP: usize = 50;

/// Period of the queue's expiry sweep. 1 Hz strikes a balance between
/// "user notices the toast vanishing on time" and "no needless wakeups
/// when nothing is queued" — the sweep self-terminates as soon as the
/// queue empties (see [`spawn_expiry_sweep`]).
const EXPIRY_TICK: Duration = Duration::from_secs(1);

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

        // 3. Live toast — visible above the status bar until the user
        // dismisses or the severity-specific timer fires. Read `now`
        // through GPUI's executor so the virtual clock in
        // `TestAppContext` can drive auto-dismiss deterministically.
        let now = cx.background_executor().now();
        self.error_toasts.push(report, now);

        // 4. Make sure an expiry sweep is running while toasts are
        // alive. Replacing the task drops the previous one, so we
        // never run two concurrent sweeps.
        self._error_expire_sweep = Some(spawn_expiry_sweep(cx));

        cx.notify();
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
        if self.error_toasts.dismiss_id(id) {
            cx.notify();
        }
    }

    /// Read-only accessor for the in-memory history ring. Used by
    /// tests and (future) command-palette surfaces.
    #[cfg(test)]
    pub(in crate::workspace) fn error_history(&self) -> &[ErrorReport] {
        &self.error_history
    }

    /// Read-only accessor for the live toast queue. Used by the
    /// renderer (snapshot pattern) and tests.
    #[allow(dead_code)] // Renderer wired in Step 3.4.
    pub(in crate::workspace) fn error_toasts(&self) -> &ErrorToastQueue {
        &self.error_toasts
    }
}

/// Spawn a 1 Hz expiry sweep that runs as long as the queue is
/// non-empty. The task self-terminates once `expire_tick` reports an
/// empty queue, so an idle workspace doesn't burn a wakeup per second.
fn spawn_expiry_sweep(cx: &mut Context<Workspace>) -> gpui::Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(EXPIRY_TICK).await;
            let still_alive = this
                .update(cx, |ws, cx| {
                    // Same virtual-clock source as `report_error` (D4).
                    let now = cx.background_executor().now();
                    let changed = ws.error_toasts.expire_tick(now);
                    if changed {
                        cx.notify();
                    }
                    !ws.error_toasts.is_empty()
                })
                .unwrap_or(false);
            if !still_alive {
                break;
            }
        }
    })
}
