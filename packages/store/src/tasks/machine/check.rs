//! `check`: the transition table and every precondition, without mutation.

use std::collections::HashSet;

use super::{EventKind, LifecycleKind, TaskEvent, TaskRecord, TaskRejected, TasksBook, subtasks};
use crate::tasks::TaskExecutionSource;

/// Whether `event` may leave a record in `from`. `Imported` and `Created`
/// target no existing record and are checked separately.
pub(super) fn allowed(from: LifecycleKind, event: EventKind) -> bool {
    use EventKind as E;
    use LifecycleKind as L;
    match event {
        E::Imported | E::Created => false,
        E::Edited
        | E::SubtaskAdded
        | E::SubtaskToggled
        | E::SubtaskRenamed
        | E::SubtaskDeleted
        | E::TodoMerged => true,
        E::Deleted => from == L::Backlog || from.is_terminal(),
        E::DispatchRequested => from == L::Backlog,
        E::LanePrepared => from == L::Preparing,
        E::DispatchFailed => matches!(from, L::Preparing | L::Dispatching),
        E::Dispatched | E::Undelivered => from == L::Dispatching,
        E::ExecutionSessionBound => {
            matches!(from, L::Dispatching | L::Running) || from.is_terminal()
        }
        E::SessionAttached
        | E::SessionDetached
        | E::Settled
        | E::ToolFailuresEscalated
        | E::LaneLost => from == L::Running,
        // CLI facts keep arriving after the run ends: the exit is usually
        // confirmed after the session-end hook already settled the task.
        // Before `LanePrepared` there is no run for them to describe.
        E::CliProcessObserved | E::CliTranscriptFound => !matches!(from, L::Backlog | L::Preparing),
        E::CancelRequested | E::Reconciled => {
            matches!(from, L::Preparing | L::Dispatching | L::Running)
        }
        E::Reopened => from.is_terminal(),
    }
}

pub(super) fn check(book: &TasksBook, event: &TaskEvent) -> Result<(), TaskRejected> {
    match event {
        TaskEvent::Imported { tasks } => return check_import(book, tasks),
        TaskEvent::Created { spec } if book.get(&spec.id).is_some() => {
            return Err(TaskRejected::DuplicateTask {
                id: spec.id.clone(),
            });
        }
        TaskEvent::Created { .. } => return Ok(()),
        _ => {}
    }
    let id = event.task_id().unwrap_or_default();
    let record = book
        .get(id)
        .ok_or_else(|| TaskRejected::UnknownTask { id: id.to_owned() })?;
    let from = record.lifecycle.kind();
    if !allowed(from, event.kind()) {
        return Err(TaskRejected::IllegalTransition {
            from,
            event: event.kind(),
        });
    }
    if let Some(exec) = event.exec()
        && record.current_exec() != Some(exec)
    {
        return Err(TaskRejected::StaleExecution { id: id.to_owned() });
    }
    check_payload(record, event)
}

fn check_import(book: &TasksBook, tasks: &[TaskRecord]) -> Result<(), TaskRejected> {
    if !book.is_empty() {
        return Err(TaskRejected::ImportIntoNonEmptyBook);
    }
    let mut seen = HashSet::new();
    for task in tasks {
        if !seen.insert(task.id()) {
            return Err(TaskRejected::DuplicateTask {
                id: task.id().to_owned(),
            });
        }
    }
    Ok(())
}

/// Preconditions beyond the table, on a record the event may act on.
fn check_payload(record: &TaskRecord, event: &TaskEvent) -> Result<(), TaskRejected> {
    let id = || record.id().to_owned();
    let is_cli = record
        .execution
        .as_ref()
        .is_some_and(|run| matches!(run.source, TaskExecutionSource::ClaudeCli { .. }));
    let bound = record
        .execution
        .as_ref()
        .and_then(|run| run.session_id.as_deref());
    match event {
        TaskEvent::SubtaskAdded { subtask, .. } => subtasks::check_added(record, subtask),
        TaskEvent::SubtaskToggled { subtask_id, .. }
        | TaskEvent::SubtaskDeleted { subtask_id, .. } => {
            subtasks::check_existing(record, subtask_id)
        }
        TaskEvent::SubtaskRenamed {
            subtask_id, title, ..
        } => subtasks::check_renamed(record, subtask_id, title),
        TaskEvent::TodoMerged {
            session_id, items, ..
        } => subtasks::check_merge(record, session_id, items),
        TaskEvent::Dispatched {
            session_id: Some(session_id),
            ..
        }
        | TaskEvent::ExecutionSessionBound { session_id, .. } => {
            if record.execution.is_none() {
                return Err(TaskRejected::StaleExecution { id: id() });
            }
            match bound {
                Some(bound) if bound != session_id => {
                    Err(TaskRejected::SessionAlreadyBound { id: id() })
                }
                _ => Ok(()),
            }
        }
        TaskEvent::SessionAttached { .. }
        | TaskEvent::SessionDetached { .. }
        | TaskEvent::CliProcessObserved { .. }
        | TaskEvent::CliTranscriptFound { .. }
            if !is_cli =>
        {
            Err(TaskRejected::NotCliExecution { id: id() })
        }
        TaskEvent::Reconciled { .. }
            if record.lifecycle.kind() == LifecycleKind::Running
                && is_cli
                && !record
                    .execution
                    .as_ref()
                    .and_then(|run| run.source.cli_process())
                    .is_some_and(|process| {
                        matches!(
                            process,
                            crate::tasks::CliProcessState::Unknown
                                | crate::tasks::CliProcessState::ExitConfirmed { .. }
                        )
                    }) =>
        {
            Err(TaskRejected::CliRunOwnedByTracker { id: id() })
        }
        _ => Ok(()),
    }
}
