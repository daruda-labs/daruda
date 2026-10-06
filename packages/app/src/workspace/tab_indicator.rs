//! The one dot a tab shows: the most urgent of its live session status and
//! the outcome of a turn that ended while it was out of view.

use daruda_agent::{AgentOutcome, SessionStatus};

use crate::surface::strings as s;

/// What a tab's dot says. Every state is a plain dot told apart by colour, so
/// each one also has a tooltip line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum TabIndicator {
    /// A session is blocked on the user's answer.
    Attention,
    /// A session died and cannot go on until the user acts.
    Failed,
    /// A turn ended in an error the user has not seen.
    Errored,
    /// A turn is running, a tool call included.
    Working,
    /// A turn finished and the user has not seen its answer.
    Done,
}

impl TabIndicator {
    /// `live` is the tab's aggregated session status, `unseen` the unseen
    /// outcomes of its panes. `None` when none of them has anything to say.
    pub(in crate::workspace) fn resolve(
        live: Option<SessionStatus>,
        unseen: impl IntoIterator<Item = AgentOutcome>,
    ) -> Option<Self> {
        live.and_then(Self::from_live)
            .into_iter()
            .chain(unseen.into_iter().map(Self::from_outcome))
            .max_by_key(|indicator| indicator.rank())
    }

    /// `Idle` and `Connecting` are where every agent tab rests, so they stay
    /// quiet. A tool call is still the turn working, not a separate state.
    fn from_live(status: SessionStatus) -> Option<Self> {
        match status {
            SessionStatus::NeedsAttention => Some(Self::Attention),
            SessionStatus::Failed => Some(Self::Failed),
            SessionStatus::Working | SessionStatus::ExecutingTool => Some(Self::Working),
            SessionStatus::Idle | SessionStatus::Connecting => None,
        }
    }

    fn from_outcome(outcome: AgentOutcome) -> Self {
        match outcome {
            AgentOutcome::Completed => Self::Done,
            AgentOutcome::Errored => Self::Errored,
        }
    }

    /// What wants the user soonest wins: an answer they owe, then a dead
    /// session, then an error they missed. A running turn outranks a missed
    /// answer, which is waiting for nothing.
    fn rank(self) -> u8 {
        match self {
            Self::Attention => 4,
            Self::Failed => 3,
            Self::Errored => 2,
            Self::Working => 1,
            Self::Done => 0,
        }
    }

    /// The tooltip line naming the state.
    pub(in crate::workspace) fn label(self) -> String {
        match self {
            Self::Attention => s::tab_strip::status_attention(),
            Self::Failed => s::tab_strip::status_failed(),
            Self::Errored => s::tab_strip::status_errored(),
            Self::Working => s::tab_strip::status_working(),
            Self::Done => s::tab_strip::status_done(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AgentOutcome::{Completed, Errored};
    use SessionStatus as S;

    #[test]
    fn a_resting_tab_with_nothing_unseen_shows_nothing() {
        assert_eq!(TabIndicator::resolve(None, []), None);
        assert_eq!(TabIndicator::resolve(Some(S::Idle), []), None);
        assert_eq!(TabIndicator::resolve(Some(S::Connecting), []), None);
    }

    #[test]
    fn a_tool_call_is_the_turn_working() {
        assert_eq!(
            TabIndicator::resolve(Some(S::ExecutingTool), []),
            Some(TabIndicator::Working)
        );
    }

    #[test]
    fn an_unseen_outcome_shows_on_a_resting_tab() {
        assert_eq!(
            TabIndicator::resolve(Some(S::Idle), [Completed]),
            Some(TabIndicator::Done)
        );
        assert_eq!(
            TabIndicator::resolve(None, [Errored]),
            Some(TabIndicator::Errored)
        );
    }

    #[test]
    fn an_error_outranks_a_completion_across_panes() {
        assert_eq!(
            TabIndicator::resolve(None, [Completed, Errored, Completed]),
            Some(TabIndicator::Errored)
        );
    }

    /// The whole order, pairwise: each row beats every row below it.
    #[test]
    fn the_most_urgent_state_wins() {
        let order = [
            (Some(S::NeedsAttention), None, TabIndicator::Attention),
            (Some(S::Failed), None, TabIndicator::Failed),
            (None, Some(Errored), TabIndicator::Errored),
            (Some(S::Working), None, TabIndicator::Working),
            (None, Some(Completed), TabIndicator::Done),
        ];
        for (i, (live, _, high)) in order.iter().enumerate() {
            for (_, unseen, _) in &order[i + 1..] {
                if let (Some(live), Some(unseen)) = (live, unseen) {
                    assert_eq!(
                        TabIndicator::resolve(Some(*live), [*unseen]),
                        Some(*high),
                        "{live:?} against {unseen:?}"
                    );
                }
            }
        }
        assert_eq!(
            TabIndicator::resolve(Some(S::Working), [Errored]),
            Some(TabIndicator::Errored),
            "a missed error outranks a new turn"
        );
    }
}
