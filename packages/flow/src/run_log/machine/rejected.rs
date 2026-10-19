//! Why the machine refused an event. A refusal from the engine is a bug in
//! the engine; it is reported, never acted on.

use crate::NodeId;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FlowRunRejected {
    #[error("the run has already started")]
    RunAlreadyStarted,
    #[error("the run is not running")]
    NotRunning,
    #[error("the run cannot be resumed from where it stopped")]
    NotResumable,
    #[error("`{0}` is not a node of this run")]
    UnknownNode(NodeId),
    #[error("`{node}` is in generation {current}, not {proposed}")]
    WrongGeneration {
        node: NodeId,
        current: u32,
        proposed: u32,
    },
    #[error("`{0}` already has an attempt in flight")]
    InFlight(NodeId),
    #[error("`{0}` has no attempt in flight with those numbers")]
    NotInFlight(NodeId),
    #[error("a number the log assigns was proposed with a different value")]
    WrongNumber,
    #[error("`{0}` has not passed, so it cannot lose or give up an output")]
    NotPassed(NodeId),
    #[error("`{0}` was not invalidated, so a repair has nothing to re-derive")]
    NotInvalidated(NodeId),
    #[error("`{0}` leaves no output file, so it has none to lose")]
    NoOutput(NodeId),
    #[error("`{0}` has no fix session in that state")]
    NoFix(NodeId),
    #[error("question {0} is not open")]
    NoSuchAsk(u64),
    #[error("questions {0:?} are still open")]
    OpenAsks(Vec<u64>),
}
