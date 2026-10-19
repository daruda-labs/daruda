//! The task lifecycle as a sealed state machine kept in a run log.
//!
//! [`TaskMachine`] is the only writer of a [`TasksBook`]: every change is a
//! [`TaskEvent`] that `check` accepts against the transition table, the log
//! makes durable, and `apply` replays deterministically.

mod apply;
mod book;
mod check;
mod event;
mod lifecycle;
mod rejected;

pub use book::{EndedExecution, ExecutionDescription, TaskRecord, TaskSpec, TasksBook};
pub use event::{EventKind, SettleOutcome, StartedBy, StopCause, TaskEdit, TaskEvent, TodoItem};
pub use lifecycle::{
    ErrorCause, InterruptCause, Lifecycle, LifecycleKind, StartFailure, UndeliveredReason,
};
pub use rejected::TaskRejected;

/// The task lifecycle [`daruda_runlog::Machine`]. It owns no numbers, so
/// `assign` is the default.
pub struct TaskMachine;

impl daruda_runlog::Machine for TaskMachine {
    type State = TasksBook;
    type Event = TaskEvent;
    type Rejected = TaskRejected;
    // Bump with any change to how `TaskEvent` or `TasksBook` serializes, so an older
    // build refuses the log as too new. 2 and 3 were used only by uncommitted
    // builds whose logs may exist; never reuse them.
    const SCHEMA: u32 = 4;

    fn check(state: &TasksBook, event: &TaskEvent) -> Result<(), TaskRejected> {
        check::check(state, event)
    }

    fn apply(state: &mut TasksBook, event: &TaskEvent) {
        apply::apply(state, event);
    }
}

#[cfg(test)]
mod tests;
