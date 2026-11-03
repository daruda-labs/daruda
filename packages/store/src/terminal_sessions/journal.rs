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
        if self.0.should_compact() {
            self.0.compact()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_journal_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let journal = SessionJournal::open(dir.path()).unwrap();
        assert!(journal.state().executions.is_empty());
        drop(journal);
        assert!(
            SessionJournal::read(dir.path())
                .unwrap()
                .executions
                .is_empty()
        );
    }

    #[test]
    fn a_corrupt_journal_is_refused_without_overwriting_it() {
        let dir = tempfile::tempdir().unwrap();
        drop(SessionJournal::open(dir.path()).unwrap());
        let path = FileBackend::new(dir.path()).log_path(0);
        std::fs::write(&path, b"corrupt\ncorrupt\n").unwrap();
        assert!(SessionJournal::read(dir.path()).is_err());
        assert!(SessionJournal::open(dir.path()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"corrupt\ncorrupt\n");
    }

    #[test]
    fn newer_schemas_are_refused_without_overwriting_them() {
        struct FutureMachine;
        impl daruda_runlog::Machine for FutureMachine {
            type State = super::super::SessionHistory;
            type Event = super::super::SessionEvent;
            type Rejected = std::convert::Infallible;
            const SCHEMA: u32 = 2;
            fn check(_: &Self::State, _: &Self::Event) -> Result<(), Self::Rejected> {
                Ok(())
            }
            fn apply(_: &mut Self::State, _: &Self::Event) {}
        }
        let dir = tempfile::tempdir().unwrap();
        drop(RunLog::<FutureMachine, _>::open(FileBackend::new(dir.path()), "future").unwrap());
        let path = FileBackend::new(dir.path()).log_path(0);
        let before = std::fs::read(&path).unwrap();
        assert!(matches!(
            SessionJournal::open(dir.path()),
            Err(HistoryError::Open(daruda_runlog::OpenError::TooNew { .. }))
        ));
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}
