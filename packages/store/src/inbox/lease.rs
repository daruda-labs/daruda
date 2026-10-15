//! The original per-bot OS lock, retained until its inbox writer drains.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct InboxLease {
    _file: File,
    directory: PathBuf,
}

impl InboxLease {
    /// Acquire the existing `bot-<digest>.lock`, or return `None` when
    /// another process owns it. `bot` must be the credential's opaque digest.
    /// The original file handle is never cloned or independently unlocked.
    pub fn try_acquire(remote_locks: &Path, bot: &str) -> io::Result<Option<Self>> {
        let directory = crate::storage::StorageLayout::inbox_at(remote_locks, bot)?;
        daruda_core::path::create_owner_only_dir(remote_locks)?;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(remote_locks.join(format!("bot-{bot}.lock")))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self {
                _file: file,
                directory,
            })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    pub(super) fn directory(&self) -> &Path {
        &self.directory
    }
}

#[cfg(test)]
mod tests {}
