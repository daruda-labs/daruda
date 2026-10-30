//! The stop reason a finished `session/prompt` reported, read once.
//!
//! [`crate::AcpEvent::TurnEnded`] carries the reason as its Debug-formatted
//! text. Every consumer that has to tell a finished answer from a turn the
//! agent cut short reads it through [`TurnStop::from_reported`], so a second
//! `match` on the same strings cannot drift from this one.

/// Why a prompt turn ended, as the agent reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStop {
    /// The agent finished its answer.
    EndTurn,
    /// The client cancelled the turn.
    Cancelled,
    /// The agent stopped before finishing, for a reason it named.
    Early(EarlyStop),
    /// A reason this build does not know, kept whole so it can still be shown.
    Unrecognized(String),
}

/// A turn the agent ended on its own before its answer was done. Reported as a
/// normal completion on the wire, so nothing else marks the answer as partial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EarlyStop {
    /// The answer hit the model's output-token limit.
    MaxTokens,
    /// The agent used up the requests it may make in one turn.
    MaxTurnRequests,
    /// The model declined to continue.
    Refusal,
}

impl TurnStop {
    /// Read the `stop_reason` text of a [`crate::AcpEvent::TurnEnded`].
    #[must_use]
    pub fn from_reported(stop_reason: &str) -> Self {
        match stop_reason {
            "EndTurn" => Self::EndTurn,
            "Cancelled" => Self::Cancelled,
            "MaxTokens" => Self::Early(EarlyStop::MaxTokens),
            "MaxTurnRequests" => Self::Early(EarlyStop::MaxTurnRequests),
            "Refusal" => Self::Early(EarlyStop::Refusal),
            other => Self::Unrecognized(other.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The texts are the Debug names of the protocol's `StopReason`, which is
    /// what `session/turn.rs` formats into the event. A rename upstream fails
    /// here instead of turning every early stop into `Unrecognized`.
    #[test]
    fn reported_texts_are_the_protocol_debug_names() {
        use agent_client_protocol::schema::v1::StopReason;
        for (reason, expected) in [
            (StopReason::EndTurn, TurnStop::EndTurn),
            (StopReason::Cancelled, TurnStop::Cancelled),
            (StopReason::MaxTokens, TurnStop::Early(EarlyStop::MaxTokens)),
            (
                StopReason::MaxTurnRequests,
                TurnStop::Early(EarlyStop::MaxTurnRequests),
            ),
            (StopReason::Refusal, TurnStop::Early(EarlyStop::Refusal)),
        ] {
            assert_eq!(TurnStop::from_reported(&format!("{reason:?}")), expected);
        }
    }
}
