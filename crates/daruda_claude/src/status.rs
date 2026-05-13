//! Shared status types — produced by both the hook FSM
//! ([`crate::hooks::fsm`]) and the JSONL fallback FSM
//! ([`crate::jsonl::fsm`]).

use serde::{Deserialize, Serialize};

/// User-facing Claude session status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub enum SessionStatus {
    /// Claude is generating a response (model inference in progress).
    Working,
    /// A tool call is actively executing (`PreToolUse` received,
    /// `PostToolUse` / `PostToolUseFailure` not yet seen).
    ExecutingTool,
    /// Waiting for the user — permission prompt, idle prompt, elicitation.
    NeedsAttention,
    /// Idle — turn ended, waiting for the next user prompt.
    Idle,
    /// Session is starting up or status is unknown (no events yet).
    Connecting,
}

impl SessionStatus {
    /// Aggregate priority for collapsing N session statuses into a single
    /// indicator (e.g. one worktree row showing the "worst" of multiple
    /// concurrent Claude sessions).
    ///
    /// Higher number = higher priority = wins the aggregate slot.
    /// `NeedsAttention > ExecutingTool = Working > Idle > Connecting`.
    pub fn priority(self) -> u8 {
        match self {
            Self::NeedsAttention => 3,
            Self::ExecutingTool | Self::Working => 2,
            Self::Idle => 1,
            Self::Connecting => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_ordering() {
        assert!(SessionStatus::NeedsAttention.priority() > SessionStatus::Working.priority());
        assert!(SessionStatus::NeedsAttention.priority() > SessionStatus::ExecutingTool.priority());
        assert_eq!(
            SessionStatus::Working.priority(),
            SessionStatus::ExecutingTool.priority()
        );
        assert!(SessionStatus::Working.priority() > SessionStatus::Idle.priority());
        assert!(SessionStatus::Idle.priority() > SessionStatus::Connecting.priority());
    }

    #[test]
    fn aggregate_picks_highest_priority() {
        let states = [
            SessionStatus::Idle,
            SessionStatus::Working,
            SessionStatus::Connecting,
        ];
        let max = states.iter().copied().max_by_key(|s| s.priority()).unwrap();
        assert_eq!(max, SessionStatus::Working);

        let states2 = [SessionStatus::ExecutingTool, SessionStatus::Idle];
        let max2 = states2
            .iter()
            .copied()
            .max_by_key(|s| s.priority())
            .unwrap();
        assert_eq!(max2, SessionStatus::ExecutingTool);
    }

    #[test]
    fn json_roundtrip() {
        for s in [
            SessionStatus::Working,
            SessionStatus::ExecutingTool,
            SessionStatus::NeedsAttention,
            SessionStatus::Idle,
            SessionStatus::Connecting,
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let back: SessionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }
}
