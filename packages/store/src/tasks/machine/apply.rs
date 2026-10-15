//! `apply`: the effect of an event `check` accepted. Deterministic — every
//! timestamp and id comes from the event.

use chrono::{DateTime, Utc};

use super::{
    ErrorCause, InterruptCause, Lifecycle, SettleOutcome, TaskEdit, TaskEvent, TaskRecord,
    TasksBook, TodoItem, UndeliveredReason,
};
use crate::tasks::{ExecutionRef, SessionEndReason, SubTask};

pub(super) fn apply(book: &mut TasksBook, event: &TaskEvent) {
    match event {
        TaskEvent::Imported { tasks } => {
            for task in tasks {
                book.insert(task.clone());
            }
        }
        TaskEvent::Created { spec } => book.insert(TaskRecord {
            spec: spec.clone(),
            lifecycle: Lifecycle::Backlog,
            execution: None,
            session_ids: Vec::new(),
            subtasks: Vec::new(),
            updated_at: spec.created_at,
            finished_at: None,
        }),
        TaskEvent::Deleted { id, .. } => book.remove(id),
        _ => {
            // `check` accepted the event, so its task exists.
            if let Some(record) = event.task_id().and_then(|id| book.get_mut(id)) {
                apply_to(record, event);
            }
        }
    }
}

fn apply_to(record: &mut TaskRecord, event: &TaskEvent) {
    match event {
        TaskEvent::Imported { .. } | TaskEvent::Created { .. } | TaskEvent::Deleted { .. } => {}
        TaskEvent::Edited { edit, at, .. } => {
            edit_spec(record, edit);
            record.updated_at = *at;
        }
        TaskEvent::SubtaskAdded { subtask, at, .. } => {
            record.subtasks.push(SubTask {
                title: subtask.title.trim().to_owned(),
                ..subtask.clone()
            });
            record.updated_at = *at;
        }
        TaskEvent::SubtaskToggled { subtask_id, at, .. } => {
            if let Some(sub) = record.subtasks.iter_mut().find(|s| s.id == *subtask_id) {
                sub.completed = !sub.completed;
                record.updated_at = *at;
            }
        }
        TaskEvent::SubtaskRenamed {
            subtask_id,
            title,
            at,
            ..
        } => {
            let title = title.trim();
            if let Some(sub) = record.subtasks.iter_mut().find(|s| s.id == *subtask_id)
                && sub.title != title
            {
                title.clone_into(&mut sub.title);
                record.updated_at = *at;
            }
        }
        TaskEvent::SubtaskDeleted { subtask_id, at, .. } => {
            record.subtasks.retain(|s| s.id != *subtask_id);
            record.updated_at = *at;
        }
        TaskEvent::TodoMerged {
            session_id,
            items,
            at,
            ..
        } => {
            if merge_todos(&mut record.subtasks, session_id, items, *at) {
                record.updated_at = *at;
            }
        }
        TaskEvent::DispatchRequested { id, execution, at } => {
            record.lifecycle = Lifecycle::Preparing {
                exec: ExecutionRef {
                    task_id: id.clone(),
                    execution_id: execution.id.clone(),
                },
            };
            record.execution = Some(execution.clone());
            record.updated_at = *at;
        }
        TaskEvent::LanePrepared { exec, worktree, at } => {
            record.lifecycle = Lifecycle::Dispatching {
                exec: exec.clone(),
                worktree: worktree.clone(),
            };
            record.updated_at = *at;
        }
        TaskEvent::DispatchFailed { exec, cause, at } => {
            let worktree = record.lifecycle.worktree().map(ToOwned::to_owned);
            let cause = ErrorCause::StartFailed(*cause);
            finish(record, error(exec, worktree, cause), *at);
        }
        TaskEvent::Dispatched {
            exec,
            session_id,
            at,
        } => {
            if let Some(worktree) = record.lifecycle.worktree().map(ToOwned::to_owned) {
                record.lifecycle = Lifecycle::Running {
                    exec: exec.clone(),
                    worktree,
                };
            }
            if let Some(session_id) = session_id {
                bind_session(record, session_id);
            }
            record.updated_at = *at;
        }
        TaskEvent::Undelivered { exec, reason, at } => {
            let worktree = record.lifecycle.worktree().map(ToOwned::to_owned);
            let next = undelivered(exec, worktree, *reason);
            finish(record, next, *at);
        }
        TaskEvent::ExecutionSessionBound { session_id, at, .. } => {
            if bind_session(record, session_id) {
                record.updated_at = *at;
            }
        }
        TaskEvent::SessionAttached { session_id, at, .. } => {
            if !record.session_ids.contains(session_id) {
                record.session_ids.push(session_id.clone());
                record.updated_at = *at;
            }
        }
        TaskEvent::SessionDetached { session_id, at, .. } => {
            let before = record.session_ids.len();
            record.session_ids.retain(|s| s != session_id);
            if record.session_ids.len() != before {
                record.updated_at = *at;
            }
        }
        TaskEvent::CliProcessObserved { process, at, .. } => {
            let cli = record.execution.as_mut().and_then(|r| r.source.cli_mut());
            if let Some((_, current)) = cli
                && current != process
            {
                current.clone_from(process);
                record.updated_at = *at;
            }
        }
        TaskEvent::CliTranscriptFound {
            transcript_path,
            at,
            ..
        } => {
            let cli = record.execution.as_mut().and_then(|r| r.source.cli_mut());
            if let Some((current, _)) = cli
                && current.as_ref() != Some(transcript_path)
            {
                *current = Some(transcript_path.clone());
                record.updated_at = *at;
            }
        }
        TaskEvent::Settled { exec, outcome, at } => {
            if let Some(worktree) = record.lifecycle.worktree().map(ToOwned::to_owned) {
                let next = settle(exec, worktree, outcome);
                finish(record, next, *at);
            }
        }
        TaskEvent::ToolFailuresEscalated { exec, at } => {
            let worktree = record.lifecycle.worktree().map(ToOwned::to_owned);
            finish(record, error(exec, worktree, ErrorCause::ToolFailures), *at);
        }
        TaskEvent::CancelRequested { exec, at } => {
            let worktree = record.lifecycle.worktree().map(ToOwned::to_owned);
            let next = Lifecycle::Cancelled {
                exec: exec.clone(),
                worktree,
            };
            finish(record, next, *at);
        }
        TaskEvent::LaneLost { exec, at } => {
            if let Some(worktree) = record.lifecycle.worktree().map(ToOwned::to_owned) {
                let next = interrupted(exec, worktree, InterruptCause::LaneGone);
                finish(record, next, *at);
            }
        }
        TaskEvent::Reconciled { exec, at } => {
            let worktree = record.lifecycle.worktree().map(ToOwned::to_owned);
            let next = match (&record.lifecycle, worktree) {
                (Lifecycle::Running { .. }, Some(worktree)) => {
                    interrupted(exec, worktree, InterruptCause::AppRestart)
                }
                (_, worktree) => undelivered(exec, worktree, UndeliveredReason::AppRestart),
            };
            finish(record, next, *at);
        }
        TaskEvent::Reopened { run_in, at, .. } => {
            record.lifecycle = Lifecycle::Backlog;
            record.spec.run_in = run_in.clone();
            record.execution = None;
            record.session_ids.clear();
            record.finished_at = None;
            record.updated_at = *at;
        }
    }
}

/// Launch fields only matter when a Backlog task starts; once started they
/// name the lane the run created, so later edits leave them alone.
fn edit_spec(record: &mut TaskRecord, edit: &TaskEdit) {
    let spec = &mut record.spec;
    spec.title.clone_from(&edit.title);
    spec.prompt.clone_from(&edit.prompt);
    spec.notes.clone_from(&edit.notes);
    spec.auto_execute = edit.auto_execute;
    spec.agent_surface = edit.agent_surface;
    spec.base_worktree_path.clone_from(&edit.base_worktree_path);
    if record.lifecycle == Lifecycle::Backlog {
        if let Some(branch) = &edit.branch {
            spec.branch_name.clone_from(branch);
        }
        if let Some(run_in) = &edit.run_in {
            spec.run_in = run_in.clone();
        }
        if let Some(project) = edit.project {
            spec.project = project;
        }
    }
}

/// The `TodoWrite` merge: a row folds into an existing auto row only when
/// both title and session agree, so manual rows and other sessions' rows are
/// never touched. Titles are not rewritten, and dropped items stay.
fn merge_todos(
    subtasks: &mut Vec<SubTask>,
    session_id: &str,
    items: &[TodoItem],
    at: DateTime<Utc>,
) -> bool {
    let mut changed = false;
    for item in items {
        let title = item.content.trim();
        if title.is_empty() {
            continue;
        }
        let existing = subtasks
            .iter_mut()
            .find(|s| s.source_session_id.as_deref() == Some(session_id) && s.title == title);
        match existing {
            Some(existing) if existing.completed == item.completed => {}
            Some(existing) => {
                existing.completed = item.completed;
                changed = true;
            }
            None => {
                subtasks.push(SubTask {
                    id: item.id.clone(),
                    title: title.to_owned(),
                    completed: item.completed,
                    created_at: Some(at),
                    source_session_id: Some(session_id.to_owned()),
                });
                changed = true;
            }
        }
    }
    changed
}

/// Name the run's session once; `true` when it was unset.
fn bind_session(record: &mut TaskRecord, session_id: &str) -> bool {
    match record.execution.as_mut() {
        Some(run) if run.session_id.is_none() => {
            run.session_id = Some(session_id.to_owned());
            true
        }
        _ => false,
    }
}

/// Every terminal state ends the run's sessions and stamps its end.
fn finish(record: &mut TaskRecord, next: Lifecycle, at: DateTime<Utc>) {
    record.lifecycle = next;
    record.session_ids.clear();
    record.finished_at = Some(at);
    record.updated_at = at;
}

fn settle(exec: &ExecutionRef, worktree: std::path::PathBuf, outcome: &SettleOutcome) -> Lifecycle {
    match outcome {
        SettleOutcome::Completed {
            end_reason: SessionEndReason::Error,
        }
        | SettleOutcome::Errored => error(exec, Some(worktree), ErrorCause::SessionFailed),
        SettleOutcome::Completed { end_reason } => Lifecycle::Done {
            exec: exec.clone(),
            worktree,
            end_reason: *end_reason,
        },
        SettleOutcome::Stopped { cause } => interrupted(exec, worktree, (*cause).into()),
    }
}

fn error(
    exec: &ExecutionRef,
    worktree: Option<std::path::PathBuf>,
    cause: ErrorCause,
) -> Lifecycle {
    Lifecycle::Error {
        exec: Some(exec.clone()),
        worktree,
        cause,
    }
}

fn undelivered(
    exec: &ExecutionRef,
    worktree: Option<std::path::PathBuf>,
    reason: UndeliveredReason,
) -> Lifecycle {
    Lifecycle::Undelivered {
        exec: exec.clone(),
        worktree,
        reason,
    }
}

fn interrupted(
    exec: &ExecutionRef,
    worktree: std::path::PathBuf,
    cause: InterruptCause,
) -> Lifecycle {
    Lifecycle::Interrupted {
        exec: Some(exec.clone()),
        worktree,
        cause,
    }
}
