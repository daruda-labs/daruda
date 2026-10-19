//! Startup settling of the runs a previous process left active.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::tasks::{
    CommitError, ExecutionRef, Lifecycle, ReconcileBlocked, TaskEvent, TaskExecutionSource,
    TaskStore, TasksBook,
};

use super::ReadOnlyCause;

/// Settle every run the previous process left starting or running in a
/// chat: nothing of it survived. A CLI run is its tracker's to settle.
/// Reports how it ended; `Err` says why tasks stay read-only.
///
/// A failure handled here ends the pass and clears its marker: only a crash
/// in the middle of it may leave the marker that keeps later starts from
/// reconciling. A record the machine refuses is skipped and reported: the
/// refusal leaves the log as it was, and the other records still need
/// settling.
pub(super) fn reconcile(store: &TaskStore) -> Result<(), ReadOnlyCause> {
    use crate::surface::strings::error;
    let claim = match store.begin_reconcile() {
        Ok(claim) => claim,
        Err(ReconcileBlocked::Unfinished) => {
            report_reconcile(error::tasks_read_only(), None, "tasks.reconcile.unfinished");
            return Err(ReadOnlyCause::UnfinishedReconciliation);
        }
        // No marker was written, so the next start tries again.
        Err(ReconcileBlocked::Io(e)) => {
            report_reconcile(
                error::tasks_settle_failed(),
                Some(&e),
                "tasks.reconcile.begin",
            );
            return Err(ReadOnlyCause::SettleFailed);
        }
    };
    match settle_lost_runs(store) {
        // The runs are settled and the log is sound, so this session writes;
        // only the next start would open read-only on the leftover marker.
        Ok(()) => {
            if let Err(e) = claim.finish() {
                report_reconcile(
                    "Tasks will open read-only on the next start: the settling marker could not be removed".to_string(),
                    Some(&e),
                    "tasks.reconcile.finish",
                );
            }
            Ok(())
        }
        Err(settle) => match claim.abandon() {
            Ok(()) => {
                report_reconcile(
                    error::tasks_settle_failed(),
                    Some(&settle),
                    "tasks.reconcile.io",
                );
                Err(ReadOnlyCause::SettleFailed)
            }
            Err(cleared) => {
                LogWriter::log(
                    ErrorReport::new(error::tasks_settle_marker_stuck())
                        .severity(ErrorSeverity::Error)
                        .from_error(&cleared)
                        .with_context("settle_error", settle.to_string())
                        .at(file!(), line!())
                        .dedup("tasks.reconcile.abandon")
                        .build(),
                );
                Err(ReadOnlyCause::SettleMarkerStuck)
            }
        },
    }
}

#[track_caller]
fn report_reconcile(title: String, error: Option<&dyn std::error::Error>, dedup: &str) {
    let location = std::panic::Location::caller();
    let mut report = ErrorReport::new(title).severity(ErrorSeverity::Error);
    if let Some(error) = error {
        report = report.from_error(error);
    }
    LogWriter::log(
        report
            .at(location.file(), location.line())
            .dedup(dedup)
            .build(),
    );
}

fn settle_lost_runs(store: &TaskStore) -> Result<(), ReconcileBlocked> {
    let (_, book) = store
        .current()
        .map_err(|e| ReconcileBlocked::Io(std::io::Error::other(e)))?;
    settle(store, lost_runs(&book))
}

/// The runs a previous process left that no one will ever end.
fn lost_runs(book: &TasksBook) -> Vec<ExecutionRef> {
    book.records()
        .filter(|record| match &record.lifecycle {
            Lifecycle::Preparing { .. } | Lifecycle::Dispatching { .. } => true,
            Lifecycle::Running { .. } => !matches!(
                record.execution.as_ref().map(|run| &run.source),
                Some(TaskExecutionSource::ClaudeCli { .. })
            ),
            _ => false,
        })
        .filter_map(|record| record.current_exec().cloned())
        .collect()
}

fn settle(store: &TaskStore, lost: Vec<ExecutionRef>) -> Result<(), ReconcileBlocked> {
    let at = chrono::Utc::now();
    for exec in lost {
        let event = TaskEvent::Reconciled {
            exec: exec.clone(),
            at,
        };
        match smol::block_on(store.propose(event)) {
            Ok(_) => {}
            Err(CommitError::Rejected(rejected)) => LogWriter::log(
                ErrorReport::new("A task the last session left running could not be settled and was left as recorded")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&rejected)
                    .with_context("task", exec.task_id.to_string())
                    .at(file!(), line!())
                    .dedup("tasks.reconcile.rejected")
                    .build(),
            ),
            Err(e) => return Err(ReconcileBlocked::Io(std::io::Error::other(e))),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::no_hook;
    use crate::runs::seed::{self, SeedEnd, SeedRun};
    use daruda_store::project::ProjectUuid;

    /// A run the log refuses to settle is skipped, not a reason to stop: the
    /// runs after it still need settling, and the refusal changed nothing.
    #[test]
    fn a_refused_settle_is_skipped_and_the_rest_are_still_settled() {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::in_directory(dir.path(), None, "test", no_hook()).unwrap();
        let mut lost = Vec::new();
        for name in ["refused", "lost"] {
            let spec = seed::spec(ProjectUuid::new(), name);
            let id = spec.id.clone();
            let mut events = seed::created(spec);
            events.extend(seed::run(&id, &SeedRun::chat("/lane"), SeedEnd::Preparing));
            for event in events {
                smol::block_on(store.propose(event)).unwrap();
            }
            lost.push(id);
        }
        let (_, book) = store.current().unwrap();
        let execs = lost_runs(&book);
        assert_eq!(execs.len(), 2);
        // Settling the first run now makes the pass's own attempt a refusal.
        settle(&store, vec![execs[0].clone()]).unwrap();

        settle(&store, execs).unwrap();

        let (_, book) = store.current().unwrap();
        for id in &lost {
            assert!(matches!(
                book.get(id).unwrap().lifecycle,
                Lifecycle::Undelivered { .. }
            ));
        }
    }
}
