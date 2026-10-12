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
pub(super) fn agent_outcome(outcome: &TurnOutcome) -> Option<AgentOutcome> {
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

    const ACTIVITIES: [ActivityState; 3] = [
        ActivityState::Idle,
        ActivityState::Working,
        ActivityState::AwaitingPermission,
    ];

    /// Until the pane is connected its activity has nothing to say.
    #[test]
    fn a_pane_not_yet_connected_reads_the_same_whatever_its_activity() {
        let failed = AgentSessionStatus::Error {
            message: "adapter exited".into(),
            remedy: daruda_acp::Remedy::NoneAvailable,
        };
        for (status, expected) in [
            (AgentSessionStatus::Idle, None),
            (failed, Some(SessionStatus::Failed)),
            (
                AgentSessionStatus::PreparingRuntime(RuntimePrepPhase::Downloading),
                Some(SessionStatus::Connecting),
            ),
            (
                AgentSessionStatus::Connecting,
                Some(SessionStatus::Connecting),
            ),
            (
                AgentSessionStatus::Handshaking(ConnectPhase::Handshaking),
                Some(SessionStatus::Connecting),
            ),
        ] {
            for activity in ACTIVITIES {
                assert_eq!(
                    session_status_of(&status, activity),
                    expected,
                    "{status:?} / {activity:?}"
                );
            }
        }
    }

    #[test]
    fn a_connected_pane_reads_as_its_activity() {
        for (activity, expected) in ACTIVITIES.into_iter().zip([
            SessionStatus::Idle,
            SessionStatus::Working,
            SessionStatus::NeedsAttention,
        ]) {
            assert_eq!(
                session_status_of(&AgentSessionStatus::Connected, activity),
                Some(expected),
                "{activity:?}"
            );
        }
    }

    #[test]
    fn a_stop_leaves_nothing_to_report() {
        for (outcome, expected) in [
            (TurnOutcome::Completed, Some(AgentOutcome::Completed)),
            (TurnOutcome::Errored, Some(AgentOutcome::Errored)),
            (TurnOutcome::Stopped, None),
        ] {
            assert_eq!(agent_outcome(&outcome), expected, "{outcome:?}");
        }
    }
}
