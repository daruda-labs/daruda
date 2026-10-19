//! [`TaskStore`]: the task repository, a run log of [`TaskMachine`] events.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use daruda_runlog::{
    AfterCommit, Body, CatchUp, CommitError, Committed, Committer, FileBackend, FlushTimedOut,
    Machine as _, OpenError, Record, RunLog, SubscribeError,
};

use super::machine::{TaskEvent, TaskMachine, TaskRejected, TasksBook};
use super::migrate::migrate_legacy;
use super::persistence::load_tasks_in;

type Live = smol::channel::Receiver<std::sync::Arc<Record<TaskEvent>>>;

#[derive(Debug, thiserror::Error)]
pub enum TaskStoreError {
    #[error("task log directory could not be prepared: {0}")]
    Io(#[from] io::Error),
    #[error("task log could not be opened: {0}")]
    Open(#[from] OpenError),
    #[error("legacy tasks could not be imported: {0}")]
    Import(#[from] CommitError<TaskRejected>),
}

/// Marks a startup reconciliation in progress, inside the log directory.
const RECONCILING_SENTINEL: &str = ".reconciling";

/// Why a startup reconciliation may not run.
#[derive(Debug, thiserror::Error)]
pub enum ReconcileBlocked {
    /// A previous reconciliation never finished: it may be what crashed.
    #[error("an earlier task reconciliation did not finish")]
    Unfinished,
    #[error("task reconciliation marker could not be written: {0}")]
    Io(#[from] io::Error),
}

/// A reconciliation under way; [`Reconciling::finish`] clears its marker.
/// Dropped unfinished, the marker stays, so the next start skips it.
///
/// No `Drop` clears it on purpose: the marker exists to outlive a crash in
/// the middle of the pass, so a start that keeps crashing there stops
/// running it. Only a pass that ended — whether or not it got through —
/// says so, by [`Self::finish`] or [`Self::abandon`].
#[must_use]
pub struct Reconciling {
    sentinel: PathBuf,
}

impl Reconciling {
    /// The pass got through.
    pub fn finish(self) -> io::Result<()> {
        std::fs::remove_file(&self.sentinel)
    }

    /// The pass stopped on a failure it handled and reported. Nothing
    /// crashed, so the next start may try again rather than open the tasks
    /// read-only for good.
    ///
    /// The same removal as `finish`, kept separate so each call site states
    /// how its pass ended; the marker only records that it did end.
    pub fn abandon(self) -> io::Result<()> {
        std::fs::remove_file(&self.sentinel)
    }
}

/// Task persistence. Domain code holds this, never the log's directory.
#[derive(Clone, Debug)]
pub struct TaskStore {
    access: TaskStoreAccess,
    log_dir: std::sync::Arc<PathBuf>,
}

#[derive(Clone, Debug)]
enum TaskStoreAccess {
    Writable(Committer<TaskMachine>),
    ReadOnly {
        seq: u64,
        book: std::sync::Arc<TasksBook>,
    },
}

impl TaskStore {
    /// Open the profile's task log, importing the profile's `tasks.json` on
    /// first use. The caller must hold the profile's desktop instance lock:
    /// the log assumes a single writer.
    pub fn open_current(
        build: &str,
        after_commit: AfterCommit<TaskMachine>,
    ) -> Result<Self, TaskStoreError> {
        let layout = crate::storage::StorageLayout::current();
        let legacy = layout.data();
        Self::in_directory(layout.task_log(), Some(&legacy), build, after_commit)
    }

    /// Open the log in `log_dir`; when it holds no events yet, import
    /// `tasks.json` from `legacy_data_dir`. The source file is only read.
    /// An unfinished reconciliation opens a read-only snapshot, preserving
    /// every recovery file and skipping import.
    /// The caller guarantees no other writer has `log_dir` open.
    pub fn in_directory(
        log_dir: impl Into<PathBuf>,
        legacy_data_dir: Option<&Path>,
        build: &str,
        after_commit: AfterCommit<TaskMachine>,
    ) -> Result<Self, TaskStoreError> {
        let log_dir = log_dir.into();
        if log_dir.join(RECONCILING_SENTINEL).try_exists()? {
            let (seq, book) = RunLog::<TaskMachine, _>::read_only(&FileBackend::new(&log_dir))?;
            return Ok(Self {
                access: TaskStoreAccess::ReadOnly {
                    seq,
                    book: std::sync::Arc::new(book),
                },
                log_dir: std::sync::Arc::new(log_dir),
            });
        }
        std::fs::create_dir_all(&log_dir)?;
        let mut log = RunLog::<TaskMachine, _>::open(FileBackend::new(log_dir.clone()), build)?;
        // Committed before the committer exists, so the hook never sees the
        // import; subscribers receive it through their catch-up.
        if let Some(dir) = legacy_data_dir
            && !holds_events(&log)?
            && let Some(legacy) = load_tasks_in(dir)
        {
            log.commit(migrate_legacy(&legacy))?;
        }
        let committer = Committer::spawn(log, "task-log", after_commit)?;
        Ok(Self {
            access: TaskStoreAccess::Writable(committer),
            log_dir: std::sync::Arc::new(log_dir),
        })
    }

    /// The last committed seq and the state it left, replayed from the log.
    /// After a write failure stops the committer, the state is read from the
    /// files: nothing writes them any more, and a failed commit must not hide
    /// every task the log already holds.
    pub fn current(&self) -> Result<(u64, TasksBook), SubscribeError> {
        let (catch_up, _live) = match self.subscribe(0) {
            Ok(subscribed) => subscribed,
            Err(SubscribeError::Stopped) => {
                return RunLog::<TaskMachine, _>::read_only(&FileBackend::new(
                    self.log_dir.as_path(),
                ))
                .map_err(|e| SubscribeError::Io(io::Error::other(e)));
            }
            Err(e) => return Err(e),
        };
        Ok(match catch_up {
            CatchUp::Snapshot { state, seq } => (seq, state),
            CatchUp::Records(records) => {
                let mut book = TasksBook::default();
                let mut seq = 0;
                for record in records {
                    seq = record.seq;
                    if let Body::Event { event } = &record.body {
                        TaskMachine::apply(&mut book, event);
                    }
                }
                (seq, book)
            }
        })
    }

    /// Claim the one startup reconciliation; refused while an earlier one's
    /// marker is still on disk.
    pub fn begin_reconcile(&self) -> Result<Reconciling, ReconcileBlocked> {
        if matches!(self.access, TaskStoreAccess::ReadOnly { .. }) {
            return Err(ReconcileBlocked::Unfinished);
        }
        let sentinel = self.log_dir.join(RECONCILING_SENTINEL);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&sentinel)
        {
            Ok(_) => Ok(Reconciling { sentinel }),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                Err(ReconcileBlocked::Unfinished)
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Commit `event`; resolves once it is durable and published. A read-only
    /// recovery snapshot refuses every proposal with `CommitError::Closed`.
    pub fn propose(
        &self,
        event: TaskEvent,
    ) -> impl Future<Output = Result<Committed<TaskMachine>, CommitError<TaskRejected>>> + use<>
    {
        let commit = match &self.access {
            TaskStoreAccess::Writable(committer) => Some(committer.propose(event)),
            TaskStoreAccess::ReadOnly { .. } => None,
        };
        async move {
            match commit {
                Some(commit) => commit.await,
                None => Err(CommitError::Closed),
            }
        }
    }

    /// Everything after `after_seq`, then every later commit live. A read-only
    /// recovery snapshot supplies its state and an already-closed stream.
    pub fn subscribe(
        &self,
        after_seq: u64,
    ) -> Result<(CatchUp<TaskMachine>, Live), SubscribeError> {
        match &self.access {
            TaskStoreAccess::Writable(committer) => committer.subscribe(after_seq),
            TaskStoreAccess::ReadOnly { seq, book } => {
                let catch_up = if after_seq < *seq {
                    CatchUp::Snapshot {
                        state: book.as_ref().clone(),
                        seq: *seq,
                    }
                } else {
                    CatchUp::Records(Vec::new())
                };
                let (_, live) = smol::channel::unbounded();
                Ok((catch_up, live))
            }
        }
    }

    /// Wait until every earlier proposal is answered.
    pub fn flush(&self, deadline: Duration) -> Result<(), FlushTimedOut> {
        match &self.access {
            TaskStoreAccess::Writable(committer) => committer.flush(deadline),
            TaskStoreAccess::ReadOnly { .. } => Ok(()),
        }
    }
}

/// Whether any event was ever committed: an import runs only before the
/// first, so deleting every task never triggers a second import.
fn holds_events(log: &RunLog<TaskMachine, FileBackend>) -> io::Result<bool> {
    Ok(match log.since(0)? {
        CatchUp::Snapshot { .. } => true,
        CatchUp::Records(records) => records
            .iter()
            .any(|record| matches!(record.body, daruda_runlog::Body::Event { .. })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reconciliation_left_unfinished_blocks_the_next() {
        let temp = tempfile::tempdir().unwrap();
        let hook = || -> AfterCommit<TaskMachine> { Box::new(|_, _| {}) };
        let store = TaskStore::in_directory(temp.path(), None, "b", hook()).unwrap();
        let first = store.begin_reconcile().unwrap();
        assert!(matches!(
            store.begin_reconcile(),
            Err(ReconcileBlocked::Unfinished)
        ));
        first.finish().unwrap();
        store.begin_reconcile().unwrap().finish().unwrap();
    }

    /// A pass that failed without crashing leaves the next start free to
    /// reconcile; one that crashed (never ended) leaves it read-only.
    #[test]
    fn only_a_pass_that_never_ended_blocks_the_next_start() {
        let temp = tempfile::tempdir().unwrap();
        let hook = || -> AfterCommit<TaskMachine> { Box::new(|_, _| {}) };
        let store = TaskStore::in_directory(temp.path(), None, "b", hook()).unwrap();
        store.begin_reconcile().unwrap().abandon().unwrap();
        drop(store);
        let next = TaskStore::in_directory(temp.path(), None, "b", hook()).unwrap();
        let crashed = next.begin_reconcile().unwrap();
        std::mem::forget(crashed);
        drop(next);
        let after_crash = TaskStore::in_directory(temp.path(), None, "b", hook()).unwrap();
        assert!(matches!(
            after_crash.begin_reconcile(),
            Err(ReconcileBlocked::Unfinished)
        ));
    }

    #[test]
    fn current_replays_every_commit() {
        let temp = tempfile::tempdir().unwrap();
        let hook = || -> AfterCommit<TaskMachine> { Box::new(|_, _| {}) };
        let store = TaskStore::in_directory(temp.path(), None, "b", hook()).unwrap();
        let spec = super::super::machine::TaskSpec {
            id: "t".into(),
            project: crate::project::ProjectUuid::new(),
            title: "t".into(),
            prompt: String::new(),
            notes: String::new(),
            base_worktree_path: None,
            branch_name: "b".into(),
            run_in: Default::default(),
            agent_type: Default::default(),
            agent_surface: Default::default(),
            auto_execute: true,
            created_at: chrono::Utc::now(),
        };
        let committed = smol::block_on(store.propose(TaskEvent::Created { spec })).unwrap();
        let (seq, book) = store.current().unwrap();
        assert_eq!(seq, committed.seq);
        assert_eq!(book, committed.state_after);
    }

    #[test]
    fn a_stopped_committer_still_reads_the_durable_state() {
        let temp = tempfile::tempdir().unwrap();
        let store = TaskStore::in_directory(
            temp.path(),
            None,
            "b",
            Box::new(|_, _| panic!("the committer stops after this commit")),
        )
        .unwrap();
        let spec = super::super::machine::TaskSpec {
            id: "t".into(),
            project: crate::project::ProjectUuid::new(),
            title: "t".into(),
            prompt: String::new(),
            notes: String::new(),
            base_worktree_path: None,
            branch_name: "b".into(),
            run_in: Default::default(),
            agent_type: Default::default(),
            agent_surface: Default::default(),
            auto_execute: true,
            created_at: chrono::Utc::now(),
        };
        assert!(smol::block_on(store.propose(TaskEvent::Created { spec })).is_err());
        let (_, book) = store.current().unwrap();
        assert!(book.get("t").is_some(), "the durable record vanished");
    }

    #[test]
    fn a_fresh_directory_holds_no_events() {
        let temp = tempfile::tempdir().unwrap();
        let log = RunLog::<TaskMachine, _>::open(FileBackend::new(temp.path()), "b").unwrap();
        assert!(!holds_events(&log).unwrap());
    }
}
