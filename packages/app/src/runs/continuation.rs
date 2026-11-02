//! A person continuing the conversation of a task run that already ended: a
//! new run of the same task, in the pane it happens in.
//!
//! The ended run is left as it ended. The continuation is its own run, so its
//! result, its relay and its watch entry follow the rules of any run started
//! by that person, and the task's own result stays in its history.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::tasks::machine::{TaskEvent, TaskRecord};
use daruda_store::tasks::{ExecutionRef, StartedBy};
use gpui::App;

use super::{GlobalTasks, RunPane, RunsHost};

impl RunsHost {
    /// A person's prompt went out in `pane`, on `session_id`. When it
    /// continues a task run that ended there, record a new run of that task
    /// and bind the pane to it, so the turn it starts settles the task again.
    ///
    /// Waits for pending task writes first: the previous turn's settle is
    /// proposed at its completion edge and may not have landed yet.
    pub(crate) fn person_prompt_sent(
        pane: RunPane,
        session_id: Option<String>,
        started_by: StartedBy,
        prompt: String,
        cx: &mut App,
    ) {
        Self::after_projection(
            move |cx| {
                let Some(prior) = continuable_run(pane, session_id.as_deref(), cx) else {
                    return;
                };
                let next = ExecutionRef {
                    task_id: prior.task_id.clone(),
                    execution_id: uuid::Uuid::new_v4().to_string(),
                };
                // Bound before the write so the turn already on the wire ends
                // against the new run: its settle waits for this write to
                // land. A refused write puts the binding back.
                Self::bind_execution(pane, next.clone(), cx);
                let commit = Self::propose(
                    TaskEvent::Continued {
                        prior: prior.clone(),
                        execution_id: next.execution_id.clone(),
                        started_by,
                        prompt,
                        at: chrono::Utc::now(),
                    },
                    cx,
                );
                cx.spawn(async move |cx| {
                    let Err(error) = commit.await else {
                        return;
                    };
                    cx.update(|cx| {
                        if Self::pane_execution(pane, cx) == Some(&next) {
                            Self::bind_execution(pane, prior, cx);
                        }
                    });
                    LogWriter::log(
                        ErrorReport::new("A task continuation was not recorded")
                            .severity(ErrorSeverity::Info)
                            .message(error.to_string())
                            .at(file!(), line!())
                            .dedup("tasks.continuation_refused")
                            .build(),
                    );
                })
                .detach();
            },
            cx,
        );
    }
}

/// The ended run a prompt in `pane` continues, if any. The pane's own binding
/// decides when it has one; a pane restored after a restart has none, and is
/// matched by the session the run recorded instead.
fn continuable_run(pane: RunPane, session_id: Option<&str>, cx: &App) -> Option<ExecutionRef> {
    let tasks = cx.global::<GlobalTasks>();
    if let Some(bound) = RunsHost::pane_execution(pane, cx) {
        let record = tasks.get(&bound.task_id)?;
        return (record.current_exec() == Some(bound) && continuable(record, session_id))
            .then(|| bound.clone());
    }
    let session_id = session_id?;
    tasks
        .records()
        .filter(|record| {
            continuable(record, Some(session_id))
                && record
                    .execution
                    .as_ref()
                    .and_then(|run| run.session_id.as_deref())
                    == Some(session_id)
        })
        .max_by_key(|record| record.updated_at)
        .and_then(|record| record.current_exec().cloned())
}

/// Whether `record`'s last run can be continued on `session_id`, when one is
/// known: the task log's own rule, on the same conversation.
fn continuable(record: &TaskRecord, session_id: Option<&str>) -> bool {
    record.is_continuable()
        && session_id.is_none_or(|session| {
            record
                .execution
                .as_ref()
                .and_then(|run| run.session_id.as_deref())
                == Some(session)
        })
}

#[cfg(test)]
mod tests;
