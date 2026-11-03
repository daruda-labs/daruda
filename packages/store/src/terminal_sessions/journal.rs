//! Strict replay and persistence of one externally locked terminal journal.

use std::path::Path;

use daruda_runlog::{FileBackend, RunLog};

use super::{SessionEvent, SessionHistory, SessionMachine};

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("terminal history I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("terminal history open: {0}")]
    Open(#[from] daruda_runlog::OpenError),
    #[error("terminal history commit: {0}")]
    Commit(#[from] daruda_runlog::CommitError<std::convert::Infallible>),
}

// A hook reopens its journal for every event, so replay needs a smaller
// budget than a long-lived writer. Compaction preserves the complete state.
const COMPACT_AFTER_RECORDS: u64 = 64;

pub struct SessionJournal(RunLog<SessionMachine, FileBackend>);

impl SessionJournal {
    /// The caller holds the terminal's hook lock for this journal's lifetime.
    pub fn open(directory: &Path) -> Result<Self, HistoryError> {
        std::fs::create_dir_all(directory)?;
        Ok(Self(RunLog::open(
            FileBackend::new(directory),
            env!("CARGO_PKG_VERSION"),
        )?))
    }

    /// Readers may inspect a running writer without acquiring its lock.
    pub fn read(directory: &Path) -> Result<SessionHistory, HistoryError> {
        Ok(RunLog::<SessionMachine, _>::read_only(&FileBackend::new(directory))?.1)
    }

    pub fn state(&self) -> &SessionHistory {
        self.0.state()
    }

    pub fn commit(&mut self, event: SessionEvent) -> Result<(), HistoryError> {
        self.0.commit(event)?;
        if self.0.should_compact_after_records(COMPACT_AFTER_RECORDS) {
            self.0.compact()?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
