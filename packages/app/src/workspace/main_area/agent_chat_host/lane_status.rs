//! A chat pane's state in the vocabulary the lane indicator and the agent
//! notices speak — `daruda_agent`'s, shared with terminal panes — rather than
//! the pane's own.

use daruda_agent::{AgentOutcome, SessionStatus};
use daruda_chat::view::{ActivityState, AgentChatView, AgentSessionStatus, TurnOutcome};

/// The lane indicator's status for a pane. `None` for a dormant `Idle`, which
/// has nothing to show; a pane in `Error` shows as failed rather than dropping
/// off the lane row.
pub(in crate::workspace) fn session_status(view: &AgentChatView) -> Option<SessionStatus> {
    session_status_of(view.status(), view.activity_state())
}

fn session_status_of(
    status: &AgentSessionStatus,
    activity: ActivityState,
) -> Option<SessionStatus> {
    match status {
        AgentSessionStatus::Idle => None,
        AgentSessionStatus::Error { .. } => Some(SessionStatus::Failed),
        // Runtime prep and the handshake are both connecting sub-phases —
        // same pulsing badge.
        AgentSessionStatus::PreparingRuntime(_)
        | AgentSessionStatus::Connecting
        | AgentSessionStatus::Handshaking(_) => Some(SessionStatus::Connecting),
        AgentSessionStatus::Connected => Some(match activity {
            ActivityState::AwaitingPermission => SessionStatus::NeedsAttention,
            ActivityState::Working => SessionStatus::Working,
            ActivityState::Idle => SessionStatus::Idle,
        }),
    }
}

/// What a finished turn leaves for the user to look at; `None` for a Stop,
/// which is the user's own doing.
pub(in crate::workspace) fn agent_outcome(outcome: &TurnOutcome) -> Option<AgentOutcome> {
    match outcome {
        TurnOutcome::Completed => Some(AgentOutcome::Completed),
        TurnOutcome::Errored => Some(AgentOutcome::Errored),
        TurnOutcome::Stopped => None,
    }
}

#[cfg(test)]
mod tests {
    use daruda_acp::ConnectPhase;
    use daruda_chat::view::RuntimePrepPhase;

    use super::*;

    #[test]
    fn each_pane_state_reads_as_its_lane_status() {
        use ActivityState::{AwaitingPermission, Idle, Working};
        let failed = AgentSessionStatus::Error {
            message: "adapter exited".into(),
            remedy: daruda_acp::Remedy::NoneAvailable,
        };
        for (status, activity, expected) in [
            (AgentSessionStatus::Idle, Working, None),
            (failed, Idle, Some(SessionStatus::Failed)),
            (
                AgentSessionStatus::PreparingRuntime(RuntimePrepPhase::Downloading),
                Idle,
                Some(SessionStatus::Connecting),
            ),
            (
                AgentSessionStatus::Connecting,
                Idle,
                Some(SessionStatus::Connecting),
            ),
            (
                AgentSessionStatus::Handshaking(ConnectPhase::Handshaking),
                Idle,
                Some(SessionStatus::Connecting),
            ),
            (
                AgentSessionStatus::Connected,
                Working,
                Some(SessionStatus::Working),
            ),
            (
                AgentSessionStatus::Connected,
                AwaitingPermission,
                Some(SessionStatus::NeedsAttention),
            ),
            (
                AgentSessionStatus::Connected,
                Idle,
                Some(SessionStatus::Idle),
            ),
        ] {
            assert_eq!(
                session_status_of(&status, activity),
                expected,
                "{status:?} / {activity:?}"
            );
        }
    }

    #[test]
    fn a_stop_leaves_nothing_to_report() {
        assert_eq!(
            agent_outcome(&TurnOutcome::Completed),
            Some(AgentOutcome::Completed)
        );
        assert_eq!(
            agent_outcome(&TurnOutcome::Errored),
            Some(AgentOutcome::Errored)
        );
        assert_eq!(agent_outcome(&TurnOutcome::Stopped), None);
    }
}
