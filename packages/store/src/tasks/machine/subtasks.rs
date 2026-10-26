//! Checklist validation and deterministic edits, including session-owned TodoWrite merges.

use std::collections::HashSet;

use chrono::{DateTime, Utc};

use super::{TaskRecord, TaskRejected, TodoItem};
use crate::tasks::SubTask;

pub(super) fn check_added(record: &TaskRecord, subtask: &SubTask) -> Result<(), TaskRejected> {
    if subtask.title.trim().is_empty() {
        return Err(TaskRejected::BlankSubtaskTitle {
            id: record.id().to_owned(),
        });
    }
    if record.subtasks.iter().any(|sub| sub.id == subtask.id) {
        return Err(TaskRejected::DuplicateSubtask {
            id: record.id().to_owned(),
            subtask_id: subtask.id.clone(),
        });
    }
    Ok(())
}

pub(super) fn check_existing(record: &TaskRecord, subtask_id: &str) -> Result<(), TaskRejected> {
    if record.subtasks.iter().any(|sub| sub.id == subtask_id) {
        Ok(())
    } else {
        Err(TaskRejected::UnknownSubtask {
            id: record.id().to_owned(),
            subtask_id: subtask_id.to_owned(),
        })
    }
}

pub(super) fn check_renamed(
    record: &TaskRecord,
    subtask_id: &str,
    title: &str,
) -> Result<(), TaskRejected> {
    check_existing(record, subtask_id)?;
    if title.trim().is_empty() {
        return Err(TaskRejected::BlankSubtaskTitle {
            id: record.id().to_owned(),
        });
    }
    Ok(())
}

pub(super) fn check_merge(
    record: &TaskRecord,
    session_id: &str,
    items: &[TodoItem],
) -> Result<(), TaskRejected> {
    if !record
        .session_ids
        .iter()
        .any(|session| session == session_id)
    {
        return Err(TaskRejected::UnknownSession {
            id: record.id().to_owned(),
            session_id: session_id.to_owned(),
        });
    }
    let mut fresh = HashSet::new();
    for item in items.iter().filter(|item| !item.content.trim().is_empty()) {
        let taken = record.subtasks.iter().any(|sub| sub.id == item.id);
        if taken || !fresh.insert(item.id.as_str()) {
            return Err(TaskRejected::DuplicateSubtask {
                id: record.id().to_owned(),
                subtask_id: item.id.clone(),
            });
        }
    }
    Ok(())
}

pub(super) fn add(subtasks: &mut Vec<SubTask>, subtask: &SubTask) {
    subtasks.push(SubTask {
        title: subtask.title.trim().to_owned(),
        ..subtask.clone()
    });
}

pub(super) fn toggle(subtasks: &mut [SubTask], subtask_id: &str) -> bool {
    if let Some(sub) = subtasks.iter_mut().find(|s| s.id == subtask_id) {
        sub.completed = !sub.completed;
        true
    } else {
        false
    }
}

pub(super) fn rename(subtasks: &mut [SubTask], subtask_id: &str, title: &str) -> bool {
    let title = title.trim();
    if let Some(sub) = subtasks.iter_mut().find(|s| s.id == subtask_id)
        && sub.title != title
    {
        title.clone_into(&mut sub.title);
        true
    } else {
        false
    }
}

pub(super) fn delete(subtasks: &mut Vec<SubTask>, subtask_id: &str) {
    subtasks.retain(|s| s.id != subtask_id);
}

/// The `TodoWrite` merge: a row folds into an existing auto row only when
/// both title and session agree, so manual rows and other sessions' rows are
/// never touched. Titles are not rewritten, and dropped items stay.
pub(super) fn merge_todos(
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

#[cfg(test)]
mod tests {}
