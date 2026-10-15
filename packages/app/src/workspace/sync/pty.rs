//! Window projections of events from the process-owned PTY tracker.

use gpui::Context;

use crate::workspace::Workspace;
use daruda_agent::pty_tracker::PtyTrackerEvent;

impl Workspace {
    /// Apply one tracker event. Updates `pty_claude_bindings` and the owning
    /// task run for `BindingChanged`. `SessionProcessExited` is the one proof
    /// a CLI is gone (its PID left the OS table), so it confirms the task
    /// run's exit and sweeps the session's status file.
    pub fn apply_pty_tracker_event(&mut self, event: PtyTrackerEvent, cx: &mut Context<Self>) {
        match event {
            PtyTrackerEvent::BindingChanged { pane_id, binding } => {
                if let Some(binding) = &binding {
                    let transcript = self
                        .claude_sessions
                        .status
                        .get(&binding.session_id)
                        .and_then(|file| file.transcript_path.clone());
                    self.record_task_cli_transcript(&binding.session_id, transcript.as_deref(), cx);
                }
                let changed = match binding {
                    Some(new) => {
                        let prev = self.claude_sessions.pty_bindings.get(&pane_id);
                        let same = prev == Some(&new);
                        if !same {
                            self.claude_sessions.pty_bindings.insert(pane_id, new);
                        }
                        !same
                    }
                    None => self.claude_sessions.pty_bindings.remove(&pane_id).is_some(),
                };
                if changed {
                    // `pty_claude_bindings` change → `agent_active_session_id`
                    // in the left dock snapshot changes; a workspace render
                    // re-stages the dock and the staging diff invalidates the
                    // `.cached()` left dock. A bound session also counts
                    // toward the Dock badge.
                    self.notify_status_docks(cx);
                }
            }
            PtyTrackerEvent::SessionProcessExited {
                session_id,
                claude_pid: _,
            } => {
                self.claude_sessions
                    .last_pushed_notification
                    .remove(&session_id);
                if self.claude_sessions.status.remove(&session_id).is_some() {
                    if let Ok(dir) = daruda_agent::hooks::status_file::default_dir() {
                        use daruda_agent::hooks::status_file as sf;
                        let _ = sf::delete(&sf::path_for(&dir, &session_id));
                        // `claude` is gone (the OS no longer has its PID),
                        // so no NEW hook can spawn for this
                        // session. At most one straggler hook subprocess may
                        // still hold the flock; POSIX unlink keeps its open
                        // fd valid, and a single writer needs no
                        // serialization — safe to sweep now rather than
                        // waiting for the cold-restore TTL pass.
                        let _ = sf::delete(&sf::lock_path_for(&dir, &session_id));
                    }
                    self.notify_status_docks(cx);
                }
            }
        }
    }
}
