//! Why the machine refused an event. A rejection writes nothing.

use super::{EventKind, LifecycleKind};
use crate::tasks::TaskId;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TaskRejected {
    #[error("no task {id}")]
    UnknownTask { id: TaskId },
    #[error("task {id} already exists")]
    DuplicateTask { id: TaskId },
    #[error("tasks can only be imported into an empty book")]
    ImportIntoNonEmptyBook,
    #[error("{event:?} is not allowed from {from:?}")]
    IllegalTransition {
        from: LifecycleKind,
        event: EventKind,
    },
    /// The event names an earlier (or no) execution of the task.
    #[error("task {id}: event is for an execution that is not the current one")]
    StaleExecution { id: TaskId },
    #[error("task {id}: the current execution is not a CLI run")]
    NotCliExecution { id: TaskId },
    /// The CLI process tracker, not app-start cleanup, settles a CLI run.
    #[error("task {id}: a running CLI execution is left to its process tracker")]
    CliRunOwnedByTracker { id: TaskId },
    #[error("task {id}: the execution is already bound to another session")]
    SessionAlreadyBound { id: TaskId },
    #[error("task {id} has no session {session_id}")]
    UnknownSession { id: TaskId, session_id: String },
    #[error("task {id} has no subtask {subtask_id}")]
    UnknownSubtask { id: TaskId, subtask_id: String },
    #[error("task {id} already has a subtask {subtask_id}")]
    DuplicateSubtask { id: TaskId, subtask_id: String },
    #[error("task {id}: a subtask title cannot be blank")]
    BlankSubtaskTitle { id: TaskId },
}
