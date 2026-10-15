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
    AfterCommit, CommitError, Lifecycle, ReconcileBlocked, TaskEvent, TaskExecutionSource,
    TaskMachine, TaskRecord, TaskRejected, TaskStore, TaskStoreError, TasksBook,
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

/// Why a proposed task event did not commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TaskCommitError {
    Rejected(TaskRejected),
    /// An earlier startup reconciliation did not finish; nothing is written
    /// until the user restarts with that resolved.
    ReadOnly,
    Unavailable,
    Failed(String),
}

impl std::fmt::Display for TaskCommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(rejected) => rejected.fmt(f),
            Self::ReadOnly => {
                f.write_str("the task log is read-only after an unfinished startup reconciliation")
            }
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
    },
    Unavailable,
}

/// The process-wide owner of the task log.
pub(crate) struct RunsHost {
    access: Access,
    pub(super) delivery: super::delivery::Delivery,
    pub(super) cli: Option<super::cli::CliTracker>,
    pub(super) effects: Option<Rc<dyn RunsEffects>>,
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
            Err(ReconcileBlocked::Unfinished) => {
                LogWriter::log(
                    ErrorReport::new(crate::surface::strings::error::tasks_read_only())
                        .severity(ErrorSeverity::Error)
                        .at(file!(), line!())
                        .dedup("tasks.reconcile.unfinished")
                        .build(),
                );
                Access::ReadOnly {
                    _store: store.clone(),
                }
            }
            Err(ReconcileBlocked::Io(e)) => {
                LogWriter::log(
                    ErrorReport::new(crate::surface::strings::error::tasks_read_only())
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .dedup("tasks.reconcile.io")
                        .build(),
                );
                Access::ReadOnly {
                    _store: store.clone(),
                }
            }
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
            Access::ReadOnly { .. } => return gpui::Task::ready(Err(TaskCommitError::ReadOnly)),
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
                    TaskCommitError::ReadOnly | TaskCommitError::Unavailable => {
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

    /// Wait for every queued task commit to become durable before quitting.
    pub(crate) fn install_quit_flush(cx: &mut App) {
        cx.on_app_quit(|cx| {
            if let Access::Writable(store) = &cx.global::<RunsHost>().access
                && let Err(error) = store.flush(QUIT_FLUSH_DEADLINE)
            {
                LogWriter::log(
                    ErrorReport::new("Task commits were still queued at quit")
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

    /// Install the opened host and its projection.
    pub(crate) fn install(opened: OpenedRuns, cx: &mut App) {
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
fn reconcile(store: &TaskStore) -> Result<(), ReconcileBlocked> {
    let claim = store.begin_reconcile()?;
    let (_, book) = store
        .current()
        .map_err(|e| ReconcileBlocked::Io(std::io::Error::other(e)))?;
    let at = chrono::Utc::now();
    for record in book.records() {
        let Some(exec) = record.current_exec() else {
            continue;
        };
        let stale = match &record.lifecycle {
            Lifecycle::Preparing { .. } | Lifecycle::Dispatching { .. } => true,
            Lifecycle::Running { .. } => !matches!(
                record.execution.as_ref().map(|run| &run.source),
                Some(TaskExecutionSource::ClaudeCli { .. })
            ),
            _ => false,
        };
        if !stale {
            continue;
        }
        let event = TaskEvent::Reconciled {
            exec: exec.clone(),
            at,
        };
        if let Err(e) = smol::block_on(store.propose(event)) {
            return Err(ReconcileBlocked::Io(std::io::Error::other(e)));
        }
    }
    claim.finish()?;
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
