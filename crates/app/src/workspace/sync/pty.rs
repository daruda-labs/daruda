//! PTY-tracker event pump — bridges the GPUI-free
//! `hooks::pty_tracker` channel into the GPUI Workspace entity.
//!
//! The tracker emits diff events (`BindingChanged` / `DeadSession`)
//! through an `mpsc::Receiver` whenever an FSEvents change in the
//! sessions directory or a pane register/unregister re-resolves
//! bindings. GPUI tasks can't `recv()` on a std channel without
//! blocking the background executor, so we drain it with a 100 ms timer
//! — `tab close` fans out one `BindingChanged` per pane plus several
//! `DeadSession` events, and a single read-per-tick would lag on bursts.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use gpui::{Context, Task};

use crate::hooks::pty_tracker::PtyTrackerEvent;
use crate::workspace::Workspace;

/// 100 ms strikes a balance: short enough that visible state (sub-row
/// badges, active outline) snaps without perceptible lag, long enough
/// that the background executor isn't woken up gratuitously.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Spawn the long-lived task that pulls events from `pty_rx` and
/// applies them to the Workspace via [`Workspace::apply_pty_tracker_event`].
///
/// The returned task is held by Workspace as `_pty_event_pump`;
/// dropping it (Workspace teardown) drops the receiver, so the tracker
/// thread's next `send` fails and it shuts down cleanly.
pub(in crate::workspace) fn spawn(
    pty_rx: Receiver<PtyTrackerEvent>,
    cx: &mut Context<Workspace>,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        'outer: loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            loop {
                let ev = match pty_rx.try_recv() {
                    Ok(e) => e,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break 'outer,
                };
                if this
                    .update(cx, |ws, cx| ws.apply_pty_tracker_event(ev, cx))
                    .is_err()
                {
                    break 'outer;
                }
            }
        }
    })
}

impl Workspace {
    /// Apply one tracker event. Updates `pty_claude_bindings` for
    /// `BindingChanged`; for `DeadSession`, drops the session from
    /// the in-memory store and deletes its on-disk status file so
    /// the indicator vanishes within a poll cycle of `claude`
    /// crashing without firing `SessionEnd`.
    pub fn apply_pty_tracker_event(&mut self, event: PtyTrackerEvent, cx: &mut Context<Self>) {
        match event {
            PtyTrackerEvent::BindingChanged { pane_id, binding } => {
                let changed = match binding {
                    Some(new) => {
                        let prev = self.claude.pty_claude_bindings.get(&pane_id);
                        let same = prev == Some(&new);
                        if !same {
                            self.claude.pty_claude_bindings.insert(pane_id, new);
                        }
                        !same
                    }
                    None => self.claude.pty_claude_bindings.remove(&pane_id).is_some(),
                };
                if changed {
                    cx.notify();
                    // `pty_claude_bindings` change → `claude_active_session_id`
                    // in the left dock snapshot changes. Left dock is `.cached()`,
                    // so dirty it explicitly (Pitfall #10).
                    self.notify_left_dock(cx);
                }
            }
            PtyTrackerEvent::DeadSession { session_id } => {
                self.claude.last_pushed_notification.remove(&session_id);
                if self.claude.claude_status.remove(&session_id).is_some() {
                    if let Ok(dir) = daruda_claude::hooks::status_file::default_dir() {
                        use daruda_claude::hooks::status_file as sf;
                        let _ = sf::delete(&sf::path_for(&dir, &session_id));
                        // `claude` is gone (the tracker found no live
                        // descendant), so no NEW hook can spawn for this
                        // session. At most one straggler hook subprocess may
                        // still hold the flock; POSIX unlink keeps its open
                        // fd valid, and a single writer needs no
                        // serialization — safe to sweep now rather than
                        // waiting for the cold-restore TTL pass.
                        let _ = sf::delete(&sf::lock_path_for(&dir, &session_id));
                    }
                    cx.notify();
                    self.notify_right_dock(cx);
                    self.notify_left_dock(cx);
                }
            }
        }
    }
}
