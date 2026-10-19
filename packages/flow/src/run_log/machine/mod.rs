//! A flow run as a sealed state machine kept in a run log. It records what
//! happened; which node runs next stays the scheduler's question.

mod apply;
mod check;
mod event;
mod rejected;
mod state;

pub(crate) use event::{
    AbandonReason, FixOutcome, FlowRunEvent, Imported, ImportedAttempt, ImportedLast, ImportedNode,
    ImportedOutcome, NodeStart,
};
// Public because hosts reach them: a refused commit is named in
// `LogRefusal::Rejected`, and a started attempt carries its process group.
pub use rejected::FlowRunRejected;
pub(crate) use state::Produces;
pub use state::{AttemptState, EndKind, FlowRunState, NodeState, RunPhase, SpawnedProcess};

/// The flow run [`daruda_runlog::Machine`]. It owns the attempt, evidence and
/// question numbers, so concurrent proposers never pick the same one.
pub(crate) struct FlowRunMachine;

impl daruda_runlog::Machine for FlowRunMachine {
    type State = FlowRunState;
    type Event = FlowRunEvent;
    type Rejected = FlowRunRejected;
    // Bump with any change to how `FlowRunEvent` or `FlowRunState`
    // serializes, so an older build refuses the run's log as too new.
    const SCHEMA: u32 = 1;

    fn check(state: &FlowRunState, event: &FlowRunEvent) -> Result<(), FlowRunRejected> {
        check::check(state, event)
    }

    fn assign(state: &FlowRunState, event: &mut FlowRunEvent) {
        match event {
            FlowRunEvent::AttemptStarted {
                node,
                attempt,
                evidence,
                ..
            } => {
                *attempt = state.node(node).map_or(1, |n| n.attempts + 1);
                *evidence = check::next_evidence(state);
            }
            FlowRunEvent::NodeRefused { node, attempt, .. } => {
                *attempt = state.node(node).map_or(1, |n| n.attempts + 1);
            }
            FlowRunEvent::FixStarted { evidence, .. } => *evidence = check::next_evidence(state),
            FlowRunEvent::AskRaised { id, .. } => *id = check::next_ask(state),
            _ => {}
        }
    }

    fn apply(state: &mut FlowRunState, event: &FlowRunEvent) {
        apply::apply(state, event);
    }
}

#[cfg(test)]
mod tests;
