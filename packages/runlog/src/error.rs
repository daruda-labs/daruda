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
    /// Only from a [`Committer`](crate::Committer): a panic stopped it. The
    /// proposal that panicked may be durable; every later one gets this too.
    #[error("run log committer stopped after a panic")]
    Poisoned,
}

/// Why [`Committer::subscribe`](crate::Committer::subscribe) has no stream.
#[derive(Debug, thiserror::Error)]
pub enum SubscribeError {
    #[error("run log could not be read: {0}")]
    Io(#[from] io::Error),
    /// The log closed after a write failure, or the committer panicked.
    #[error("run log committer has stopped")]
    Stopped,
    /// From the after-commit hook: the committer would wait on itself.
    #[error("run log subscribe was called on its own committer thread")]
    OnCommitterThread,
}

/// Proposals queued before a [`Committer::flush`](crate::Committer::flush)
/// were still unanswered when its deadline passed.
#[derive(Debug, thiserror::Error)]
#[error("run log commits were still queued after {deadline:?}")]
pub struct FlushTimedOut {
    pub deadline: std::time::Duration,
}

/// Which version a log was written in that this build does not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Versioned {
    /// The record and snapshot envelope, [`FORMAT_VERSION`](crate::FORMAT_VERSION).
    Format,
    /// The machine's events and state, [`Machine::SCHEMA`](crate::Machine::SCHEMA).
    Schema,
}

impl std::fmt::Display for Versioned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Format => "format",
            Self::Schema => "schema",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// A line other than a torn last one is unreadable, its event does not
    /// fit the machine, or its seq breaks the sequence. `line` is 1-based.
    #[error("run log is corrupt at line {line}")]
    Corrupt { line: u64 },
    /// A newer build wrote it. The log is left byte-for-byte untouched.
    #[error("run log {what} {found} is newer than this build supports ({supported})")]
    TooNew {
        what: Versioned,
        found: u32,
        supported: u32,
    },
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
        assert_eq!(
            OpenError::TooNew {
                what: Versioned::Schema,
                found: 3,
                supported: 2
            }
            .to_string(),
            "run log schema 3 is newer than this build supports (2)"
        );
        let err: CommitError<std::fmt::Error> = CommitError::Closed;
        assert!(err.to_string().contains("closed"));
    }
}
