//! [`RunsHost`]: the task log's owner, and [`GlobalTasks`], its projection.
//!
//! Ordering contract: a proposal's task resolves only after the projection
//! holds the committed state, and projections apply in proposal order with a
//! `seq` guard, so an out-of-order completion never rolls state back.

use std::rc::Rc;
use std::time::Duration;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::tasks::{
    AfterCommit, CommitError, ExecutionRef, Lifecycle, ReconcileBlocked, TaskEvent,
    TaskExecutionSource, TaskMachine, TaskRecord, TaskRejected, TaskStore, TaskStoreError,
    TasksBook,
};
use gpui::{App, BorrowAppContext as _, Global};

use super::{RunsEffects, TaskIntent};

/// How long quitting waits for queued task commits to become durable.
const QUIT_FLUSH_DEADLINE: Duration = Duration::from_secs(2);

/// The read-only task projection every window renders from. Only
/// [`RunsHost`] replaces it, from a committed `state_after`.
#[derive(Default)]
pub(crate) struct GlobalTasks {
    seq: u64,
    book: TasksBook,
}

impl Global for GlobalTasks {}

impl GlobalTasks {
    pub(crate) fn get(&self, id: &str) -> Option<&TaskRecord> {
        self.book.get(id)
    }

    /// Records in creation order.
    pub(crate) fn records(&self) -> impl Iterator<Item = &TaskRecord> {
        self.book.records()
    }

    pub(crate) fn book(&self) -> &TasksBook {
        &self.book
    }

    /// Whether any run is starting or running, which drives the live tick.
    pub(crate) fn any_active(&self) -> bool {
        self.records().any(|record| {
            matches!(
                record.lifecycle,
                Lifecycle::Preparing { .. }
                    | Lifecycle::Dispatching { .. }
                    | Lifecycle::Running { .. }
            )
        })
    }
}

/// Why the task log takes no writes this session. The recovery differs, so
/// the user is told which: a restart, or removing a marker by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadOnlyCause {
    /// An earlier start stopped while settling and left its marker; that
    /// marker keeps every start read-only until it is removed by hand.
    UnfinishedReconciliation,
    /// This start could not settle the runs the last session left, and
    /// cleared its marker: the next start tries again.
    SettleFailed,
    /// As `SettleFailed`, but the marker could not be cleared, so the next
    /// start opens read-only as well.
    SettleMarkerStuck,
}

/// Why a proposed task event did not commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TaskCommitError {
    Rejected(TaskRejected),
    /// Nothing is written this session; see [`ReadOnlyCause`].
    ReadOnly(ReadOnlyCause),
    Unavailable,
    Failed(String),
}

impl std::fmt::Display for TaskCommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(rejected) => rejected.fmt(f),
            Self::ReadOnly(ReadOnlyCause::UnfinishedReconciliation) => {
                f.write_str("the task log is read-only after an unfinished startup reconciliation")
            }
            Self::ReadOnly(ReadOnlyCause::SettleFailed) => f.write_str(
                "the task log is read-only because this start could not settle lost runs",
            ),
            Self::ReadOnly(ReadOnlyCause::SettleMarkerStuck) => f.write_str(
                "the task log is read-only because this start could not settle lost runs \
                 nor clear its reconciliation marker",
            ),
            Self::Unavailable => f.write_str("the task log could not be opened"),
            Self::Failed(detail) => f.write_str(detail),
        }
    }
}

impl std::error::Error for TaskCommitError {}

enum Access {
    Writable(TaskStore),
    /// Readable, never written: see [`TaskCommitError::ReadOnly`].
    ReadOnly {
        _store: TaskStore,
        cause: ReadOnlyCause,
    },
    Unavailable,
}

/// The process-wide owner of the task log.
pub(crate) struct RunsHost {
    access: Access,
    pub(super) delivery: super::delivery::Delivery,
    pub(super) cli: Option<super::cli::CliTracker>,
    pub(super) effects: Option<Rc<dyn RunsEffects>>,
    /// The flow runs in flight; they belong to the process, not to a log.
    pub(super) flows: super::flows::FlowRuns,
    /// Closes once the newest proposal has reached the projection; the next
    /// proposal waits on it, so projections apply in proposal order.
    tail: smol::channel::Receiver<()>,
    #[cfg(any(test, feature = "screenshot"))]
    _scratch: Option<tempfile::TempDir>,
    /// Tests of what a task fact *does*, rather than of when it lands,
    /// commit each proposal on the spot.
    #[cfg(test)]
    inline: bool,
}

impl Global for RunsHost {}

/// A store opened for this process, before it is installed.
pub(crate) struct OpenedRuns {
    host: RunsHost,
    projection: GlobalTasks,
}

fn settled_tail() -> smol::channel::Receiver<()> {
    let (_, rx) = smol::channel::bounded(1);
    rx
}

impl RunsHost {
    /// Take over an opened store: reconcile the runs the previous process
    /// left active, once, and read the projection. A failed open, or a
    /// reconciliation an earlier start never finished, is reported and
    /// leaves the host unable to write rather than stopping the app.
    pub(crate) fn open(opened: Result<TaskStore, TaskStoreError>) -> OpenedRuns {
        let store = match opened {
            Ok(store) => store,
            Err(e) => {
                LogWriter::log(
                    ErrorReport::new(crate::surface::strings::error::tasks_unavailable())
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .dedup("tasks.open")
                        .build(),
                );
                return OpenedRuns::unavailable();
            }
        };
        let access = match reconcile(&store) {
            Ok(()) => Access::Writable(store.clone()),
            Err(cause) => Access::ReadOnly {
                _store: store.clone(),
                cause,
            },
        };
        let projection = match store.current() {
            Ok((seq, book)) => GlobalTasks { seq, book },
            Err(e) => {
                LogWriter::log(
                    ErrorReport::new(crate::surface::strings::error::tasks_unavailable())
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .dedup("tasks.read")
                        .build(),
                );
                return OpenedRuns::unavailable();
            }
        };
        OpenedRuns {
            host: RunsHost {
                access,
                delivery: Default::default(),
                cli: None,
                effects: None,
                flows: Default::default(),
                tail: settled_tail(),
                #[cfg(any(test, feature = "screenshot"))]
                _scratch: None,
                #[cfg(test)]
                inline: false,
            },
            projection,
        }
    }

    pub(crate) fn install_effects(effects: Rc<dyn RunsEffects>, cx: &mut App) {
        cx.update_global::<Self, _>(|host, _| host.effects = Some(effects));
    }

    pub(crate) fn task(
        intent: TaskIntent,
        cx: &mut App,
    ) -> gpui::Task<Result<u64, TaskCommitError>> {
        let cancelled = match &intent {
            TaskIntent::Cancel { exec } => Some(exec.clone()),
            _ => None,
        };
        let commit = Self::propose(intent.into_event(chrono::Utc::now()), cx);
        let Some(exec) = cancelled else { return commit };
        let effects = cx.global::<Self>().effects.clone();
        let (answer, answered) = smol::channel::bounded(1);
        cx.spawn(async move |cx| {
            let result = commit.await;
            if result.is_ok()
                && let Some(effects) = effects
            {
                cx.update(|cx| effects.stop_execution(&exec, cx));
            }
            // SILENT-OK: the command's caller stopped listening.
            let _ = answer.try_send(result);
        })
        .detach();
        cx.spawn(async move |_| {
            answered
                .recv()
                .await
                .unwrap_or(Err(TaskCommitError::Unavailable))
        })
    }

    /// Commit `event`. The task resolves after the projection reflects the
    /// commit and windows have been told; a rejection changes nothing. The
    /// commit happens whether or not the task is awaited.
    pub(crate) fn propose(
        event: TaskEvent,
        cx: &mut App,
    ) -> gpui::Task<Result<u64, TaskCommitError>> {
        #[cfg(test)]
        if cx.global::<RunsHost>().inline {
            return gpui::Task::ready(Self::commit_blocking(event, cx));
        }
        let host = cx.global::<RunsHost>();
        let store = match &host.access {
            Access::Writable(store) => store.clone(),
            Access::ReadOnly { cause, .. } => {
                return gpui::Task::ready(Err(TaskCommitError::ReadOnly(*cause)));
            }
            Access::Unavailable => return gpui::Task::ready(Err(TaskCommitError::Unavailable)),
        };
        let (done, next_tail) = smol::channel::bounded::<()>(1);
        let previous =
            cx.update_global::<RunsHost, _>(|host, _| std::mem::replace(&mut host.tail, next_tail));
        let proposal = store.propose(event);
        // The committer wakes its futures from its own thread, which gpui's
        // test scheduler cannot see; waiting on a background task keeps the
        // reply inside the executor in tests and off the UI thread in use.
        let committed = cx
            .background_executor()
            .spawn(async move { smol::block_on(proposal) });
        let (answer, answered) = smol::channel::bounded(1);
        cx.spawn(async move |cx| {
            let result = committed.await;
            // SILENT-OK: `Err` only says the previous proposal is applied.
            let _ = previous.recv().await;
            let outcome = cx.update(|cx| match result {
                Ok(committed) => {
                    let seq = committed.seq;
                    apply_projection(seq, committed.state_after, cx);
                    Ok(seq)
                }
                Err(CommitError::Rejected(rejected)) => Err(TaskCommitError::Rejected(rejected)),
                Err(e) => Err(TaskCommitError::Failed(e.to_string())),
            });
            drop(done);
            // SILENT-OK: the proposer stopped listening; the commit stands.
            let _ = answer.try_send(outcome);
        })
        .detach();
        cx.spawn(async move |_| {
            answered
                .recv()
                .await
                .unwrap_or_else(|_| Err(TaskCommitError::Failed("the app is closing".into())))
        })
    }

    /// Commit a fact the app observed (a session ended, a process exited).
    /// Callers propose only what the projection says applies; one that still
    /// loses a race is no user's mistake, so it is logged, not shown.
    pub(crate) fn propose_fact(event: TaskEvent, cx: &mut App) {
        let kind = event.kind();
        let commit = Self::propose(event, cx);
        cx.spawn(async move |_| {
            if let Err(e) = commit.await {
                let severity = match &e {
                    TaskCommitError::Rejected(_) => ErrorSeverity::Info,
                    TaskCommitError::ReadOnly(_) | TaskCommitError::Unavailable => {
                        ErrorSeverity::Warning
                    }
                    TaskCommitError::Failed(_) => ErrorSeverity::Error,
                };
                LogWriter::log(
                    ErrorReport::new("A task fact was not recorded")
                        .severity(severity)
                        .message(e.to_string())
                        .at(file!(), line!())
                        .with_context("event", format!("{kind:?}"))
                        .dedup("tasks.fact_refused")
                        .build(),
                );
            }
        })
        .detach();
    }

    /// Whether every proposal so far is in the projection, so a read now
    /// sees all of them without waiting on [`Self::barrier`].
    pub(crate) fn settled(cx: &App) -> bool {
        cx.global::<RunsHost>().tail.is_closed()
    }

    /// Resolves once every proposal made before this call has reached the
    /// projection, so a command that reads a task sees the ones before it.
    pub(crate) fn barrier(cx: &App) -> impl Future<Output = ()> + use<> {
        let tail = cx.global::<RunsHost>().tail.clone();
        async move {
            // SILENT-OK: the channel only ever closes.
            let _ = tail.recv().await;
        }
    }

    /// Wait for every queued task commit to become durable before quitting,
    /// and for running flows to end what they started.
    pub(crate) fn install_quit_flush(cx: &mut App) {
        cx.on_app_quit(|cx| {
            if !Self::shut_down_flows(QUIT_FLUSH_DEADLINE, cx) {
                LogWriter::log(
                    ErrorReport::new(crate::surface::strings::error::flows_quit_shutdown())
                        .severity(ErrorSeverity::Warning)
                        .at(file!(), line!())
                        .dedup("flows.quit_shutdown")
                        .build(),
                );
            }
            if let Access::Writable(store) = &cx.global::<RunsHost>().access
                && let Err(error) = store.flush(QUIT_FLUSH_DEADLINE)
            {
                LogWriter::log(
                    ErrorReport::new(crate::surface::strings::error::tasks_quit_flush())
                        .severity(ErrorSeverity::Warning)
                        .from_error(&error)
                        .at(file!(), line!())
                        .dedup("tasks.quit_flush")
                        .build(),
                );
            }
            async {}
        })
        .detach();
    }

    /// Install the opened host and its projection. Flow runs already in
    /// flight and the windows' effects move to the new host: they belong to
    /// the process, not to the task log being swapped.
    pub(crate) fn install(mut opened: OpenedRuns, cx: &mut App) {
        if cx.try_global::<RunsHost>().is_some() {
            let (flows, effects) = cx.update_global::<RunsHost, _>(|host, _| {
                (std::mem::take(&mut host.flows), host.effects.take())
            });
            opened.host.flows = flows;
            opened.host.effects = opened.host.effects.take().or(effects);
        }
        cx.set_global(opened.projection);
        cx.set_global(opened.host);
        Self::install_cli_tracker(cx);
    }

    /// Install a host when none was: a test gets a scratch log, anything
    /// else one that refuses writes, so a missing install is visible.
    pub(crate) fn ensure_installed(cx: &mut App) {
        if cx.has_global::<RunsHost>() {
            return;
        }
        #[cfg(test)]
        {
            Self::install(Self::scratch(Vec::new()), cx);
        }
        #[cfg(not(test))]
        {
            LogWriter::log(
                ErrorReport::new(crate::surface::strings::error::tasks_unavailable())
                    .severity(ErrorSeverity::Error)
                    .at(file!(), line!())
                    .dedup("tasks.not_installed")
                    .build(),
            );
            Self::install(OpenedRuns::unavailable(), cx);
        }
    }

    /// A log in a throwaway directory holding `records`; nothing reaches the
    /// profile data directory.
    #[cfg(any(test, feature = "screenshot"))]
    fn scratch(records: Vec<TaskRecord>) -> OpenedRuns {
        let dir = tempfile::tempdir().expect("a scratch task-log directory");
        let store = TaskStore::in_directory(dir.path(), None, env!("CARGO_PKG_VERSION"), no_hook())
            .expect("a scratch task log opens");
        if !records.is_empty() {
            smol::block_on(store.propose(TaskEvent::Imported { tasks: records }))
                .expect("an empty scratch log takes the import");
        }
        let mut opened = Self::open(Ok(store));
        opened.host._scratch = Some(dir);
        opened
    }

    /// Move the task log to a scratch copy, so a screenshot's fixtures
    /// never reach the profile's log or `tasks.json`.
    #[cfg(any(test, feature = "screenshot"))]
    pub(crate) fn suspend_persistence(cx: &mut App) {
        let records = cx.global::<GlobalTasks>().records().cloned().collect();
        let opened = Self::scratch(records);
        Self::install(opened, cx);
    }

    /// A writable task log in an isolated temporary directory.
    #[cfg(test)]
    pub(crate) fn seed_for_test(cx: &mut App) {
        Self::install(Self::scratch(Vec::new()), cx);
    }

    /// Commit `event` on this thread and project it at once: fixtures that
    /// seed inside an update cannot await.
    #[cfg(any(test, feature = "screenshot"))]
    pub(crate) fn commit_blocking(event: TaskEvent, cx: &mut App) -> Result<u64, TaskCommitError> {
        let Access::Writable(store) = &cx.global::<RunsHost>().access else {
            return Err(TaskCommitError::Unavailable);
        };
        match smol::block_on(store.propose(event)) {
            Ok(committed) => {
                apply_projection(committed.seq, committed.state_after, cx);
                Ok(committed.seq)
            }
            Err(CommitError::Rejected(rejected)) => Err(TaskCommitError::Rejected(rejected)),
            Err(e) => Err(TaskCommitError::Failed(e.to_string())),
        }
    }

    /// Commit every later proposal on the spot, for tests whose subject is
    /// what an event does rather than when it lands.
    #[cfg(test)]
    pub(crate) fn inline_commits_for_test(cx: &mut App) {
        Self::ensure_installed(cx);
        cx.update_global::<RunsHost, _>(|host, _| host.inline = true);
    }

    /// Whether proposals can commit.
    #[cfg(test)]
    pub(crate) fn is_writable(cx: &App) -> bool {
        matches!(cx.global::<RunsHost>().access, Access::Writable(_))
    }

    #[cfg(test)]
    pub(crate) fn sequence_for_test(cx: &App) -> u64 {
        cx.global::<GlobalTasks>().seq
    }
}

impl OpenedRuns {
    fn unavailable() -> Self {
        Self {
            host: RunsHost {
                access: Access::Unavailable,
                delivery: Default::default(),
                cli: None,
                effects: None,
                flows: Default::default(),
                tail: settled_tail(),
                #[cfg(any(test, feature = "screenshot"))]
                _scratch: None,
                #[cfg(test)]
                inline: false,
            },
            projection: GlobalTasks::default(),
        }
    }
}

/// The hook the task store runs after each commit: none, since the host
/// applies `state_after` itself on the UI thread.
pub(crate) fn no_hook() -> AfterCommit<TaskMachine> {
    Box::new(|_, _| {})
}

/// Settle every run the previous process left starting or running in a
/// chat: nothing of it survived. A CLI run is its tracker's to settle.
/// Reports how it ended; `Err` says why tasks stay read-only.
///
/// A failure handled here ends the pass and clears its marker: only a crash
/// in the middle of it may leave the marker that keeps later starts from
/// reconciling. A record the machine refuses is skipped and reported: the
/// refusal leaves the log as it was, and the other records still need
/// settling.
fn reconcile(store: &TaskStore) -> Result<(), ReadOnlyCause> {
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
                    error::tasks_reconcile_marker_left(),
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
                ErrorReport::new(crate::surface::strings::error::task_command_refused())
                    .severity(ErrorSeverity::Warning)
                    .from_error(&rejected)
                    .at(file!(), line!())
                    .dedup("tasks.reconcile.rejected")
                    .build(),
            ),
            Err(e) => return Err(ReconcileBlocked::Io(std::io::Error::other(e))),
        }
    }
    Ok(())
}

/// Replace the projection with a committed state no older than it, then let
/// every window know.
fn apply_projection(seq: u64, state: TasksBook, cx: &mut App) {
    let newer = cx.global::<GlobalTasks>().seq < seq;
    if !newer {
        return;
    }
    // `update_global` wakes each Workspace's `observe_global`, which
    // notifies that workspace alone.
    cx.update_global::<GlobalTasks, _>(|projection, _| {
        projection.seq = seq;
        projection.book = state;
    });
}

#[cfg(test)]
mod tests {}
