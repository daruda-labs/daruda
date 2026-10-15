//! The task log written back as a legacy `tasks.json`.
//!
//! Exists only so a downgrade to a build that predates the task log still
//! finds the user's tasks. The conversion is lossy: the old schema has no
//! word for a starting, undelivered or interrupted run.

use std::path::PathBuf;

use super::machine::{ErrorCause, Lifecycle, TaskRecord, TasksBook};
use super::{Task, TaskState, TasksState};

/// Every record in the shape an older build reads.
pub fn legacy_rows(book: &TasksBook) -> TasksState {
    TasksState {
        tasks: book.records().map(legacy_task).collect(),
        ..TasksState::default()
    }
}

fn legacy_task(record: &TaskRecord) -> Task {
    let spec = &record.spec;
    // The old schema requires a path on every finished row; an empty one
    // reads as a lane that is gone, which is what it is.
    let or_none = |worktree: &Option<PathBuf>| worktree.clone().unwrap_or_default();
    let state = match &record.lifecycle {
        // Nothing exists yet, so an older build may simply start it again.
        Lifecycle::Backlog | Lifecycle::Preparing { .. } => TaskState::Backlog,
        Lifecycle::Dispatching { worktree, .. } | Lifecycle::Running { worktree, .. } => {
            TaskState::Running {
                worktree_path: worktree.clone(),
            }
        }
        Lifecycle::Done {
            worktree,
            end_reason,
            ..
        } => TaskState::Done {
            worktree_path: worktree.clone(),
            end_reason: *end_reason,
        },
        Lifecycle::Error {
            worktree, cause, ..
        } => TaskState::Error {
            worktree_path: or_none(worktree),
            message: error_text(cause),
        },
        Lifecycle::Cancelled { worktree, .. } => TaskState::Cancelled {
            worktree_path: or_none(worktree),
        },
        Lifecycle::Undelivered {
            worktree, reason, ..
        } => TaskState::Error {
            worktree_path: or_none(worktree),
            message: format!("The prompt was not delivered ({reason:?})"),
        },
        Lifecycle::Interrupted { worktree, .. } => TaskState::Cancelled {
            worktree_path: worktree.clone(),
        },
    };
    let execution = match &record.lifecycle {
        Lifecycle::Backlog | Lifecycle::Preparing { .. } => None,
        _ => record.execution.clone(),
    };
    Task {
        id: spec.id.clone(),
        project: spec.project,
        title: spec.title.clone(),
        prompt: spec.prompt.clone(),
        state,
        session_ids: record.session_ids.clone(),
        created_at: spec.created_at,
        updated_at: record.updated_at,
        finished_at: record.finished_at,
        notes: spec.notes.clone(),
        base_worktree_path: spec.base_worktree_path.clone(),
        branch_name: spec.branch_name.clone(),
        run_in: spec.run_in.clone(),
        agent_type: spec.agent_type,
        agent_surface: spec.agent_surface,
        execution,
        auto_execute: spec.auto_execute,
        subtasks: record.subtasks.clone(),
    }
}

/// Short English text: an older build shows the message verbatim.
fn error_text(cause: &ErrorCause) -> String {
    match cause {
        ErrorCause::StartFailed(failure) => format!("The task could not start ({failure:?})"),
        ErrorCause::SessionFailed => "The session ended with an error".to_owned(),
        ErrorCause::ToolFailures => "Too many tool failures".to_owned(),
        ErrorCause::Imported { message } => message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectUuid;
    use crate::tasks::machine::{InterruptCause, TaskEvent};
    use crate::tasks::{ExecutionRef, migrate_legacy};

    fn exec(task: &Task) -> ExecutionRef {
        ExecutionRef {
            task_id: task.id.clone(),
            execution_id: "run".into(),
        }
    }

    #[test]
    fn an_interrupted_run_downgrades_as_cancelled_in_its_lane() {
        let task = Task::new(ProjectUuid::new(), "t".into(), "p".into(), None);
        let TaskEvent::Imported { mut tasks } = migrate_legacy(&TasksState {
            tasks: vec![task.clone()],
            ..TasksState::default()
        }) else {
            panic!("an import");
        };
        tasks[0].lifecycle = Lifecycle::Interrupted {
            exec: Some(exec(&task)),
            worktree: PathBuf::from("/wt"),
            cause: InterruptCause::UserStop,
        };
        let mut book = TasksBook::default();
        <super::super::TaskMachine as daruda_runlog::Machine>::apply(
            &mut book,
            &TaskEvent::Imported { tasks },
        );
        let rows = legacy_rows(&book);
        assert_eq!(
            rows.tasks[0].state,
            TaskState::Cancelled {
                worktree_path: PathBuf::from("/wt")
            }
        );
        assert_eq!(rows.tasks[0].title, "t");
    }
}
