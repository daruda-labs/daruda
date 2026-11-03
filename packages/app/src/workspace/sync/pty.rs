//! Window projections of events from the process-owned PTY tracker.

use gpui::Context;

use crate::workspace::Workspace;
use daruda_agent::pty_tracker::ProcessExited;

impl Workspace {
    /// A confirmed OS exit retires the session mirror, independently of
    /// whether its terminal or window is still present.
    pub fn apply_pty_tracker_event(&mut self, event: ProcessExited, cx: &mut Context<Self>) {
        let ProcessExited { session_id, pid } = event;
        let keys: Vec<_> = self
            .claude_sessions
            .status
            .iter()
            .filter(|(_, file)| {
                file.session_id == session_id
                    && (file.agent_pid == Some(pid)
                        || (file.terminal_id.is_none() && file.agent_pid.is_none()))
            })
            .map(|(key, _)| key.to_owned())
            .collect();
        self.claude_sessions.pty_bindings.retain(|_, binding| {
            binding.session_id != session_id
                || (binding.claude_pid != Some(pid)
                    && (binding.terminal_id.is_some() || binding.claude_pid.is_some()))
        });
        for key in &keys {
            if let daruda_agent::StatusKey::Terminal(terminal) = key {
                crate::runs::record_terminal_history(
                    terminal.clone(),
                    daruda_store::terminal_sessions::SessionEvent::ProcessExited {
                        session_id: session_id.clone(),
                        pid,
                        at: chrono::Utc::now(),
                    },
                    cx,
                );
            }
            self.forget_terminal_session(key);
        }
        if !keys.is_empty() {
            self.notify_status_docks(cx);
        }
    }
}

#[cfg(test)]
mod tests;
