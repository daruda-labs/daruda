use chrono::{DateTime, Utc};
use daruda_store::tasks::machine::TaskEdit;
use daruda_store::tasks::{
    ExecutionRef, SubTask, TaskEvent, TaskExecutionSource, TaskRunIn, TaskSpec,
};

pub(crate) enum TaskIntent {
    Create {
        spec: TaskSpec,
    },
    Edit {
        id: String,
        edit: TaskEdit,
    },
    Delete {
        id: String,
    },
    AddSubtask {
        id: String,
        subtask: SubTask,
    },
    ToggleSubtask {
        id: String,
        subtask_id: String,
    },
    RenameSubtask {
        id: String,
        subtask_id: String,
        title: String,
    },
    DeleteSubtask {
        id: String,
        subtask_id: String,
    },
    Start {
        exec: ExecutionRef,
        source: TaskExecutionSource,
    },
    Cancel {
        exec: ExecutionRef,
    },
    Reopen {
        id: String,
        run_in: TaskRunIn,
    },
}

impl TaskIntent {
    pub(super) fn into_event(self, at: DateTime<Utc>) -> TaskEvent {
        match self {
            Self::Create { spec } => TaskEvent::Created { spec },
            Self::Edit { id, edit } => TaskEvent::Edited { id, edit, at },
            Self::Delete { id } => TaskEvent::Deleted { id, at },
            Self::AddSubtask { id, subtask } => TaskEvent::SubtaskAdded { id, subtask, at },
            Self::ToggleSubtask { id, subtask_id } => {
                TaskEvent::SubtaskToggled { id, subtask_id, at }
            }
            Self::RenameSubtask {
                id,
                subtask_id,
                title,
            } => TaskEvent::SubtaskRenamed {
                id,
                subtask_id,
                title,
                at,
            },
            Self::DeleteSubtask { id, subtask_id } => {
                TaskEvent::SubtaskDeleted { id, subtask_id, at }
            }
            Self::Start { exec, source } => TaskEvent::DispatchRequested {
                id: exec.task_id,
                execution_id: exec.execution_id,
                source,
                at,
            },
            Self::Cancel { exec } => TaskEvent::CancelRequested { exec, at },
            Self::Reopen { id, run_in } => TaskEvent::Reopened { id, run_in, at },
        }
    }
}

#[cfg(test)]
mod tests {}
