//! Which hook update ends a turn, and with what outcome.
//!
//! The hook channel carries no "turn ended" signal beyond `Stop` /
//! `StopFailure`, and the watcher re-delivers files it already handed over,
//! so a turn counts as ended only when this channel also saw it open. The
//! merged status store is no witness: the JSONL fallback writes it too and
//! can settle a session to `Idle` before its `Stop` hook lands.
//!
//! A turn whose start and `Stop` reach the watcher as one coalesced read is
//! never seen open and ends without an outcome.

use std::collections::HashSet;

use crate::{AgentOutcome, SessionStatus};

/// Sessions whose turn the hook channel has seen open and not yet end.
/// Starts empty, so files re-read at startup report nothing that ended
/// before this process was watching.
#[derive(Debug, Default)]
pub struct OpenTurns {
    sessions: HashSet<String>,
}

impl OpenTurns {
    /// Fold one hook status-file update in. `status` is the status the hook
    /// FSM wrote with `last_event`. Returns the outcome of the turn this
    /// update ends; `None` for every update that ends no open turn.
    pub fn observe(
        &mut self,
        session_id: &str,
        status: SessionStatus,
        last_event: &str,
    ) -> Option<AgentOutcome> {
        let ending = match last_event {
            "Stop" => Some(AgentOutcome::Completed),
            "StopFailure" => Some(AgentOutcome::Errored),
            _ => None,
        };
        if let Some(outcome) = ending {
            return self.sessions.remove(session_id).then_some(outcome);
        }
        let busy = matches!(
            status,
            SessionStatus::Working | SessionStatus::ExecutingTool | SessionStatus::NeedsAttention
        );
        if busy {
            self.sessions.insert(session_id.to_owned());
        } else {
            self.sessions.remove(session_id);
        }
        None
    }

    /// Drop `session_id` — its status file is gone with the session.
    pub fn forget(&mut self, session_id: &str) {
        self.sessions.remove(session_id);
    }
}
