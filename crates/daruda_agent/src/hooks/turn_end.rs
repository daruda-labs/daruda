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

#[cfg(test)]
mod tests {
    use super::*;
    use SessionStatus as S;

    fn opened(event: &str, status: SessionStatus) -> OpenTurns {
        let mut turns = OpenTurns::default();
        assert_eq!(turns.observe("s", status, event), None);
        turns
    }

    #[test]
    fn a_stop_ends_a_turn_the_channel_saw_open() {
        for (event, status) in [
            ("UserPromptSubmit", S::Working),
            ("PreToolUse", S::ExecutingTool),
            ("PermissionRequest", S::NeedsAttention),
        ] {
            let mut turns = opened(event, status);
            assert_eq!(
                turns.observe("s", S::Idle, "Stop"),
                Some(AgentOutcome::Completed),
                "{event}"
            );
        }
    }

    #[test]
    fn a_stop_failure_errors_the_turn() {
        let mut turns = opened("UserPromptSubmit", S::Working);
        assert_eq!(
            turns.observe("s", S::Idle, "StopFailure"),
            Some(AgentOutcome::Errored)
        );
    }

    /// The watcher hands the same file over again; startup re-reads every
    /// file. Neither is a turn ending now.
    #[test]
    fn a_stop_with_no_open_turn_ends_nothing() {
        let mut turns = opened("UserPromptSubmit", S::Working);
        turns.observe("s", S::Idle, "Stop");
        assert_eq!(turns.observe("s", S::Idle, "Stop"), None, "re-delivered");
        assert_eq!(
            OpenTurns::default().observe("s", S::Idle, "Stop"),
            None,
            "never seen open"
        );
    }

    #[test]
    fn events_inside_a_turn_keep_it_open() {
        let mut turns = opened("UserPromptSubmit", S::Working);
        for (event, status) in [
            ("PreToolUse", S::ExecutingTool),
            ("PostToolUse", S::Working),
            ("Notification", S::Working),
        ] {
            assert_eq!(turns.observe("s", status, event), None);
        }
        assert!(turns.observe("s", S::Idle, "Stop").is_some());
    }

    /// A fresh `SessionStart` means whatever turn was open is gone.
    #[test]
    fn a_resting_update_closes_the_turn_without_an_outcome() {
        let mut turns = opened("UserPromptSubmit", S::Working);
        assert_eq!(turns.observe("s", S::Idle, "SessionStart"), None);
        assert_eq!(turns.observe("s", S::Idle, "Stop"), None);
    }

    #[test]
    fn forgetting_a_session_closes_its_turn() {
        let mut turns = opened("UserPromptSubmit", S::Working);
        turns.forget("s");
        assert_eq!(turns.observe("s", S::Idle, "Stop"), None);
    }

    #[test]
    fn sessions_are_tracked_apart() {
        let mut turns = opened("UserPromptSubmit", S::Working);
        assert_eq!(turns.observe("other", S::Idle, "Stop"), None);
        assert!(turns.observe("s", S::Idle, "Stop").is_some());
    }
}
