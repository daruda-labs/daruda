//! [`TaskStore`]: the task repository, a run log of [`TaskMachine`] events.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use daruda_runlog::{
    AfterCommit, CatchUp, CommitError, Committed, Committer, FileBackend, FlushTimedOut, OpenError,
    Record, RunLog, SubscribeError,
};

use super::machine::{TaskEvent, TaskMachine, TaskRejected};
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

/// Task persistence. Domain code holds this, never the log's directory.
#[derive(Clone, Debug)]
pub struct TaskStore {
    committer: Committer<TaskMachine>,
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
    /// The caller guarantees no other writer has `log_dir` open.
    pub fn in_directory(
        log_dir: impl Into<PathBuf>,
        legacy_data_dir: Option<&Path>,
        build: &str,
        after_commit: AfterCommit<TaskMachine>,
    ) -> Result<Self, TaskStoreError> {
        let log_dir = log_dir.into();
        std::fs::create_dir_all(&log_dir)?;
        let mut log = RunLog::<TaskMachine, _>::open(FileBackend::new(log_dir), build)?;
        // Committed before the committer exists, so the hook never sees the
        // import; subscribers receive it through their catch-up.
        if let Some(dir) = legacy_data_dir
            && !holds_events(&log)?
            && let Some(legacy) = load_tasks_in(dir)
        {
            log.commit(migrate_legacy(&legacy))?;
        }
        let committer = Committer::spawn(log, "task-log", after_commit)?;
        Ok(Self { committer })
    }

    /// Commit `event`; resolves once it is durable and published.
    pub fn propose(
        &self,
        event: TaskEvent,
    ) -> impl Future<Output = Result<Committed<TaskMachine>, CommitError<TaskRejected>>> + use<>
    {
        self.committer.propose(event)
    }

    /// Everything after `after_seq`, then every later commit live.
    pub fn subscribe(
        &self,
        after_seq: u64,
    ) -> Result<(CatchUp<TaskMachine>, Live), SubscribeError> {
        self.committer.subscribe(after_seq)
    }

    /// Wait until every earlier proposal is answered.
    pub fn flush(&self, deadline: Duration) -> Result<(), FlushTimedOut> {
        self.committer.flush(deadline)
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
    fn a_fresh_directory_holds_no_events() {
        let temp = tempfile::tempdir().unwrap();
        let log = RunLog::<TaskMachine, _>::open(FileBackend::new(temp.path()), "b").unwrap();
        assert!(!holds_events(&log).unwrap());
    }
}
