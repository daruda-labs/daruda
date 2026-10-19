//! One-time import of the legacy `tasks.json` rows into the task log.

use std::collections::HashSet;

use super::machine::{
    ErrorCause, InterruptCause, Lifecycle, LifecycleKind, TaskEvent, TaskRecord, TaskSpec,
};
use super::{ExecutionRef, Task, TaskState, TasksState};

/// The execution id given to a finished legacy row that recorded none, so
/// its terminal state still names an execution. No event can match it.
pub const LEGACY_EXECUTION_ID: &str = "legacy";

/// The `Imported` event for `state`. A row left `Running` by the old build
/// lost its process with that build, so it imports as interrupted by the
/// restart. A repeated id keeps its first row.
pub fn migrate_legacy(state: &TasksState) -> TaskEvent {
    let mut seen = HashSet::new();
    let tasks = state
        .tasks
        .iter()
        .filter(|task| seen.insert(task.id.as_str()))
        .map(migrate_task)
        .collect();
    TaskEvent::Imported { tasks }
}

fn migrate_task(task: &Task) -> TaskRecord {
    let recorded = task.execution.as_ref().map(|run| ExecutionRef {
        task_id: task.id.clone(),
        execution_id: run.id.clone(),
    });
    let or_legacy = || {
        recorded.clone().unwrap_or_else(|| ExecutionRef {
            task_id: task.id.clone(),
            execution_id: LEGACY_EXECUTION_ID.to_owned(),
        })
    };
    let lifecycle = match &task.state {
        TaskState::Backlog => Lifecycle::Backlog,
        TaskState::Running { worktree_path } => Lifecycle::Interrupted {
            exec: recorded.clone(),
            worktree: worktree_path.clone(),
            cause: InterruptCause::AppRestart,
        },
        TaskState::Done {
            worktree_path,
            end_reason,
        } => Lifecycle::Done {
            exec: or_legacy(),
            worktree: worktree_path.clone(),
            end_reason: *end_reason,
        },
        TaskState::Error {
            worktree_path,
            message,
        } => Lifecycle::Error {
            exec: recorded.clone(),
            worktree: Some(worktree_path.clone()),
            cause: ErrorCause::Imported {
                message: message.clone(),
            },
        },
        TaskState::Cancelled { worktree_path } => Lifecycle::Cancelled {
            exec: or_legacy(),
            worktree: Some(worktree_path.clone()),
        },
    };
    // The machine never produces a Backlog row with an execution or a
    // terminal row with live sessions; importing either would let events
    // (e.g. `TodoMerged`) act on a run that is over.
    let execution = if lifecycle.kind() == LifecycleKind::Backlog {
        None
    } else {
        task.execution.clone()
    };
    let session_ids = if lifecycle.is_terminal() {
        Vec::new()
    } else {
        task.session_ids.clone()
    };
    TaskRecord {
        description: None,
        started_by: None,
        started_at: None,
        ended_executions: Vec::new(),
        spec: TaskSpec {
            id: task.id.clone(),
            project: task.project,
            title: task.title.clone(),
            prompt: task.prompt.clone(),
            notes: task.notes.clone(),
            base_worktree_path: task.base_worktree_path.clone(),
            branch_name: task.branch_name.clone(),
            run_in: task.run_in.clone(),
            agent_type: task.agent_type,
            agent_surface: task.agent_surface,
            auto_execute: task.auto_execute,
            created_at: task.created_at,
        },
        lifecycle,
        execution,
        session_ids,
        subtasks: task.subtasks.clone(),
        updated_at: task.updated_at,
        finished_at: task.finished_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectUuid;

    #[test]
    fn a_repeated_legacy_id_keeps_its_first_row() {
        let first = Task::new(ProjectUuid::new(), "first".into(), String::new(), None);
        let mut second = first.clone();
        second.title = "second".into();
        let state = TasksState {
            tasks: vec![first, second],
            ..TasksState::default()
        };
        let TaskEvent::Imported { tasks } = migrate_legacy(&state) else {
            panic!("migration yields an import");
        };
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].spec.title, "first");
    }
}
