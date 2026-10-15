//! Why a commit or an open failed.

use std::io;

/// A commit that did not happen. Only `Rejected` leaves the log usable for
/// the same reason it was before; after `Io`, every commit returns `Closed`.
#[derive(Debug, thiserror::Error)]
pub enum CommitError<R> {
    #[error("{0}")]
    Rejected(R),
    #[error("run log write failed: {0}")]
    Io(#[from] io::Error),
    #[error("run log is closed after an earlier write failure")]
    Closed,
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// A line other than a torn last one is unreadable, its event does not
    /// fit the machine, or its seq breaks the sequence. `line` is 1-based.
    #[error("run log is corrupt at line {line}")]
    Corrupt { line: u64 },
    #[error("run log format {found} is newer than this build supports ({supported})")]
    TooNew { found: u32, supported: u32 },
    /// The newest generation's snapshot is unreadable and no older
    /// generation remains to fall back to. Opening empty would lose it all.
    #[error("run log generation {generation} has no whole snapshot to restore from")]
    NoWholeGeneration { generation: u64 },
    #[error("run log could not be read: {0}")]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_the_failure() {
        assert_eq!(
            OpenError::Corrupt { line: 4 }.to_string(),
            "run log is corrupt at line 4"
        );
        let err: CommitError<std::fmt::Error> = CommitError::Closed;
        assert!(err.to_string().contains("closed"));
    }
}
