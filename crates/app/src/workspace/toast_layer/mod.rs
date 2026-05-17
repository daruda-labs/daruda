//! Toast notification layer — owns the live queue, the expiry sweep
//! task, and rendering. Corresponds to `ToastLayout` in the UI hierarchy.
//!
//! `Workspace` holds a single `Entity<ToastLayer>` and delegates all
//! toast mutations through it. Rendering updates are localised to this
//! entity so a toast change never triggers a full `Workspace` repaint.

mod overlay;

use std::time::Duration;

use daruda_store::observability::error_report::ErrorReport;
use gpui::{Context, IntoElement, Render, WeakEntity, Window};

use super::Workspace;
use super::error::toast::{ErrorToastQueue, ToastId};
use self::overlay::{ErrorToastOverlay, ToastSnapshot};

/// Period of the expiry sweep. 1 Hz: timely enough that the user sees
/// the toast vanish on schedule; infrequent enough not to burn wakeups
/// while the queue is idle.
const EXPIRY_TICK: Duration = Duration::from_secs(1);

pub(in crate::workspace) struct ToastLayer {
    pub(in crate::workspace) queue: ErrorToastQueue,
    /// Running expiry sweep. Replaced (not duplicated) on every push;
    /// self-terminates when the queue empties.
    _sweep: Option<gpui::Task<()>>,
    /// Needed by the render layer to wire dismiss + details click
    /// handlers back into the Workspace.
    workspace: WeakEntity<Workspace>,
}

impl ToastLayer {
    pub(in crate::workspace) fn new(workspace: WeakEntity<Workspace>) -> Self {
        Self {
            queue: ErrorToastQueue::default(),
            _sweep: None,
            workspace,
        }
    }

    /// Push a report onto the queue and (re)start the expiry sweep.
    pub(in crate::workspace) fn push(&mut self, report: ErrorReport, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        self.queue.push(report, now);
        self._sweep = Some(spawn_expiry_sweep(cx));
        cx.notify();
    }

    /// Dismiss the toast identified by `id`. Stale ids are a silent no-op.
    pub(in crate::workspace) fn dismiss_id(&mut self, id: ToastId, cx: &mut Context<Self>) {
        if self.queue.dismiss_id(id) {
            cx.notify();
        }
    }

}

impl Render for ToastLayer {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = self.workspace.clone();
        let snapshots: Vec<ToastSnapshot> = self
            .queue
            .iter()
            .map(|t| ToastSnapshot {
                id: t.id,
                title: t.report.title.clone().into(),
                message: t.report.message.clone().into(),
                repeat_count: t.repeat_count,
                severity: t.report.severity,
                plain_text: t.report.to_plain_text().into(),
                report: t.report.clone(),
            })
            .collect();

        ErrorToastOverlay {
            toasts: snapshots,
            workspace,
        }
    }
}

fn spawn_expiry_sweep(cx: &mut Context<ToastLayer>) -> gpui::Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(EXPIRY_TICK).await;
            let still_alive = this
                .update(cx, |tl, cx| {
                    let now = cx.background_executor().now();
                    let changed = tl.queue.expire_tick(now);
                    if changed {
                        cx.notify();
                    }
                    !tl.queue.is_empty()
                })
                .unwrap_or(false);
            if !still_alive {
                break;
            }
        }
    })
}
