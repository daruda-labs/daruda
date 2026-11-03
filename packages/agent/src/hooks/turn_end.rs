//! Which hook update ends a turn, and with what outcome.
//!
//! The hook channel carries no "turn ended" signal beyond `Stop` /
//! `StopFailure`, and the watcher re-delivers files it already handed over,
//! so a turn counts as ended only when this channel also saw it open. The
//! file's last event timestamp can also be replayed by the watcher.
//!
//! A turn whose start and Stop coalesce into one read has no observed opening
//! and therefore produces no completion outcome.

use std::collections::HashSet;

use crate::{AgentOutcome, SessionStatus, StatusKey};

/// Completion semantics shared by the live hook and confirmed-exit paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookEnd {
    Completed,
    Failed,
    SessionEnded,
}

impl HookEnd {
    pub fn from_event(event: &str) -> Option<Self> {
        match event {
            "Stop" => Some(Self::Completed),
            "StopFailure" => Some(Self::Failed),
            "SessionEnd" => Some(Self::SessionEnded),
            _ => None,
        }
    }

    pub fn turn_outcome(self) -> Option<AgentOutcome> {
        match self {
            Self::Completed => Some(AgentOutcome::Completed),
            Self::Failed => Some(AgentOutcome::Errored),
            Self::SessionEnded => None,
        }
    }
}

/// Sessions whose turn the hook channel has seen open and not yet end.
/// Starts empty, so files re-read at startup report nothing that ended
/// before this process was watching.
#[derive(Debug, Default)]
pub struct OpenTurns {
    sessions: HashSet<StatusKey>,
}

impl OpenTurns {
    /// Fold one hook status-file update in. `status` is the status the hook
    /// FSM wrote with `last_event`. Returns the outcome of the turn this
    /// update ends; `None` for every update that ends no open turn.
    pub fn observe(
        &mut self,
        identity: &StatusKey,
        status: SessionStatus,
        last_event: &str,
    ) -> Option<AgentOutcome> {
        let ending = HookEnd::from_event(last_event).and_then(HookEnd::turn_outcome);
        if let Some(outcome) = ending {
            return self.sessions.remove(identity).then_some(outcome);
        }
        let busy = matches!(
            status,
            SessionStatus::Working | SessionStatus::ExecutingTool | SessionStatus::NeedsAttention
        );
        if busy {
            self.sessions.insert(identity.clone());
        } else {
            self.sessions.remove(identity);
        }
        None
    }

    /// Drop the observation whose status file is gone.
    pub fn forget(&mut self, identity: &StatusKey) {
        self.sessions.remove(identity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_goodbye_is_distinct_from_a_completed_or_failed_turn() {
        assert_eq!(
            HookEnd::from_event("Stop").and_then(HookEnd::turn_outcome),
            Some(AgentOutcome::Completed)
        );
        assert_eq!(
            HookEnd::from_event("StopFailure").and_then(HookEnd::turn_outcome),
            Some(AgentOutcome::Errored)
        );
        assert_eq!(
            HookEnd::from_event("SessionEnd"),
            Some(HookEnd::SessionEnded)
        );
        assert_eq!(HookEnd::SessionEnded.turn_outcome(), None);
        assert_eq!(HookEnd::from_event("Interrupt"), None);
        assert_eq!(HookEnd::from_event("PostToolUseFailure"), None);
    }
}
