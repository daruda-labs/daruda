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
    /// The session failed and cannot continue until the user acts.
    Failed,
}

impl SessionStatus {
    /// Aggregate priority for collapsing N session statuses into a single
    /// indicator (e.g. one lane row showing the "worst" of multiple
    /// concurrent Claude sessions).
    ///
    /// Higher number = higher priority = wins the aggregate slot.
    /// `NeedsAttention > Failed > ExecutingTool = Working > Idle > Connecting`.
    pub fn priority(self) -> u8 {
        match self {
            Self::NeedsAttention => 4,
            Self::Failed => 3,
            Self::ExecutingTool | Self::Working => 2,
            Self::Idle => 1,
            Self::Connecting => 0,
        }
    }

    /// Whether the badge for this status moves. `Idle` and `Failed` are
    /// still, so neither keeps the status-pulse clock running.
    pub fn animates(self) -> bool {
        !matches!(self, Self::Idle | Self::Failed)
    }

    /// Collapse N session statuses into the single highest-priority one
    /// (`None` for an empty input). The one shared definition of the
    /// indicator-aggregate rule — every per-lane / per-path collapse
    /// must go through this so they can never disagree. Ties between
    /// equal-priority statuses resolve to the last one in input order.
    pub fn aggregate(statuses: impl IntoIterator<Item = SessionStatus>) -> Option<SessionStatus> {
        statuses.into_iter().max_by_key(|s| s.priority())
    }
}

/// How a turn that ran to its end finished. A turn the user stopped has no
/// outcome: stopping is their own doing and leaves nothing to look at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentOutcome {
    Completed,
    Errored,
}
