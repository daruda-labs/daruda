//! Startup settling of the runs a previous process left active.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::tasks::{
    CommitError, ExecutionRef, Lifecycle, ReconcileBlocked, TaskEvent, TaskExecutionSource,
    TaskStore, TasksBook,
};

use super::ReadOnlyCause;

/// Settle every run the previous process left starting or running in a
/// chat: nothing of it survived. A CLI run is its tracker's to settle. A
/// flow run kept its own log, which says how far it got.
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

/// The task an event settles, for the log.
fn event_exec(event: &TaskEvent) -> String {
    match event {
        TaskEvent::Reconciled { exec, .. }
        | TaskEvent::LaneLost { exec, .. }
        | TaskEvent::Dispatched { exec, .. }
        | TaskEvent::DispatchFailed { exec, .. }
        | TaskEvent::Settled { exec, .. } => exec.task_id.clone(),
        _ => String::new(),
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
    let at = chrono::Utc::now();
    let mut events: Vec<TaskEvent> = lost_runs(&book)
        .into_iter()
        .map(|exec| TaskEvent::Reconciled { exec, at })
        .collect();
    events.extend(
        book.records()
            .flat_map(|record| flow_settlement(record, at)),
    );
    settle(store, events)
}

/// The runs a previous process left that no one will ever end.
fn lost_runs(book: &TasksBook) -> Vec<ExecutionRef> {
    book.records()
        .filter(|record| match &record.lifecycle {
            Lifecycle::Preparing { .. } => true,
            Lifecycle::Dispatching { .. } | Lifecycle::Running { .. } => {
                match record.execution.as_ref().map(|run| &run.source) {
                    Some(TaskExecutionSource::ClaudeCli { .. }) => {
                        matches!(record.lifecycle, Lifecycle::Dispatching { .. })
                    }
                    Some(TaskExecutionSource::Flow { .. }) => false,
                    Some(TaskExecutionSource::AgentChat) | None => true,
                }
            }
            _ => false,
        })
        .filter_map(|record| record.current_exec().cloned())
        .collect()
}

/// What a flow task's run log says about a run the last session left
/// starting or running. A run that ended settles the task; one that stalled
/// or was cut short stays running, to be resumed from the Flows panel and
/// settled when it ends; one that never wrote a log never started.
fn flow_settlement(
    record: &daruda_store::tasks::TaskRecord,
    at: chrono::DateTime<chrono::Utc>,
) -> Vec<TaskEvent> {
    let (Some(exec), Some(run)) = (record.current_exec(), record.execution.as_ref()) else {
        return Vec::new();
    };
    let Some(run_dir) = super::flow_tasks::task_run_dir(run) else {
        return Vec::new();
    };
    let dispatching = match &record.lifecycle {
        Lifecycle::Dispatching { .. } => true,
        Lifecycle::Running { .. } => false,
        _ => return Vec::new(),
    };
    let reconciled = || {
        vec![TaskEvent::Reconciled {
            exec: exec.clone(),
            at,
        }]
    };
    if !run.cwd.is_dir() {
        return if dispatching {
            reconciled()
        } else {
            vec![TaskEvent::LaneLost {
                exec: exec.clone(),
                at,
            }]
        };
    }
    if !daruda_flow::run_log::reader::started(&run_dir) {
        return reconciled();
    }
    let mut events = Vec::new();
    if dispatching {
        events.push(TaskEvent::Dispatched {
            exec: exec.clone(),
            session_id: None,
            at,
        });
    }
    let end = daruda_flow::run_log::reader::inspect(&run_dir)
        .ok()
        .flatten()
        .and_then(|inspection| inspection.end);
    if let Some(end) = end {
        let report = std::fs::read_to_string(run_dir.join(daruda_flow::record::RUN_REPORT_FILE))
            .ok()
            .and_then(|report| daruda_control_types::agent_text::bound_agent_text(&report));
        // The log holds a start, since `started` above found it.
        events.extend(super::flow_tasks::task_end(
            &end,
            false,
            true,
            report,
            exec.clone(),
            at,
        ));
    }
    events
}

fn settle(store: &TaskStore, events: Vec<TaskEvent>) -> Result<(), ReconcileBlocked> {
    for event in events {
        let exec = event_exec(&event);
        match smol::block_on(store.propose(event)) {
            Ok(_) => {}
            Err(CommitError::Rejected(rejected)) => LogWriter::log(
                ErrorReport::new("A task the last session left running could not be settled and was left as recorded")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&rejected)
                    .with_context("task", exec)
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

    fn flow_task(store: &TaskStore, lane: &std::path::Path, end: SeedEnd) -> (String, String) {
        let run_id = "0000000000000001-00000001-0001".to_string();
        let run = SeedRun {
            source: TaskExecutionSource::Flow {
                run_id: run_id.clone(),
                flow: "review.yaml".into(),
            },
            ..SeedRun::chat(lane)
        };
        let spec = seed::spec(ProjectUuid::new(), "flow");
        let id = spec.id.clone();
        let mut events = seed::created(spec);
        events.extend(seed::run(&id, &run, end));
        for event in events {
            smol::block_on(store.propose(event)).unwrap();
        }
        (id, run_id)
    }

    fn settled(store: &TaskStore, id: &str) -> Lifecycle {
        settle_lost_runs(store).unwrap();
        store
            .current()
            .unwrap()
            .1
            .get(id)
            .unwrap()
            .lifecycle
            .clone()
    }

    /// A flow run is not lost with the process that started it: its log says
    /// how far it got, and one that may still be resumed keeps its task
    /// running.
    #[test]
    fn a_flow_task_is_settled_from_its_runs_log() {
        let dir = tempfile::tempdir().unwrap();
        let lane = dir.path().join("lane");
        std::fs::create_dir_all(&lane).unwrap();
        let store =
            TaskStore::in_directory(dir.path().join("tasks"), None, "t", no_hook()).unwrap();

        // No log: the run never started.
        let (never, _) = flow_task(&store, &lane, SeedEnd::Running);
        assert!(matches!(
            settled(&store, &never),
            Lifecycle::Interrupted {
                cause: daruda_store::tasks::machine::InterruptCause::AppRestart,
                ..
            }
        ));
    }

    /// Run a one-command flow to its end in `lane` under `run_id`, as the
    /// last session would have before it quit.
    fn ended_run(lane: &std::path::Path, run_id: &str) {
        let text = "version: 1\nnodes:\n  - id: ok\n    kind: command\n    run: \"exit 0\"\n";
        let request = daruda_flow::request::RunRequest {
            history: Default::default(),
            started_by: Some(daruda_flow::request::StartedBy::Desktop),
            loaded: daruda_flow::load(text, None).unwrap(),
            cwd: lane.to_path_buf(),
            run_dir: crate::runs::runs_dir(lane).join(run_id),
            lock_dir: lane.join("..").join("locks"),
            flow_dir: lane.to_path_buf(),
            agents: Default::default(),
            node_install_dir: lane.join("..").join("node"),
            budget: daruda_flow::request::Budget::unlimited(),
            is_alive: Box::new(|_| false),
            git_status: None,
            events: None,
            ask: None,
            until: None,
            pinned: Vec::new(),
            inputs: Default::default(),
            resume: None,
        };
        let runners = daruda_flow::runner::Runners {
            command: daruda_flow::runner::ProcessRunner::new(Vec::new()),
            agent: daruda_flow::runner::AcpRunner::new(Default::default(), lane.join("node")),
        };
        let report = daruda_flow::schedule::execute(
            &request,
            &runners,
            &daruda_flow::runner::CancelToken::default(),
        );
        assert!(
            matches!(report.outcome, daruda_flow::schedule::RunOutcome::Done),
            "{report:?}"
        );
    }

    /// A run that ended while no one was watching settles its task, with
    /// the run's report as the result; one still starting is now running.
    #[test]
    fn a_flow_task_whose_run_ended_is_settled_with_its_report() {
        let dir = tempfile::tempdir().unwrap();
        let lane = dir.path().join("lane");
        std::fs::create_dir_all(&lane).unwrap();
        let store =
            TaskStore::in_directory(dir.path().join("tasks"), None, "t", no_hook()).unwrap();
        let (id, run_id) = flow_task(&store, &lane, SeedEnd::Dispatching);
        ended_run(&lane, &run_id);
        assert!(matches!(settled(&store, &id), Lifecycle::Done { .. }));
        let record = store.current().unwrap().1;
        let ended = record.get(&id).unwrap().ended_executions.last().unwrap();
        assert!(ended.result.is_some(), "the run's report is the result");
    }

    /// The worktree a flow task ran in is gone: so is its run.
    #[test]
    fn a_flow_task_whose_worktree_is_gone_lost_its_lane() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            TaskStore::in_directory(dir.path().join("tasks"), None, "t", no_hook()).unwrap();
        let (id, _) = flow_task(&store, &dir.path().join("gone"), SeedEnd::Running);
        assert!(matches!(
            settled(&store, &id),
            Lifecycle::Interrupted {
                cause: daruda_store::tasks::machine::InterruptCause::LaneGone,
                ..
            }
        ));
    }

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
        let reconciled = |exec: &ExecutionRef| TaskEvent::Reconciled {
            exec: exec.clone(),
            at: chrono::Utc::now(),
        };
        // Settling the first run now makes the pass's own attempt a refusal.
        settle(&store, vec![reconciled(&execs[0])]).unwrap();

        settle(&store, execs.iter().map(reconciled).collect()).unwrap();

        let (_, book) = store.current().unwrap();
        for id in &lost {
            assert!(matches!(
                book.get(id).unwrap().lifecycle,
                Lifecycle::Undelivered { .. }
            ));
        }
    }
}
