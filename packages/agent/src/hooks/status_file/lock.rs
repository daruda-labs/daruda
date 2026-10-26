//! Shared synchronization for hook updates and startup maintenance.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;

/// Closing the original handle releases the lock. Its pathname is retained
/// so every handler, including a queued opener, uses the same lock identity.
pub(in crate::hooks) struct SessionLock {
    _file: File,
}

impl SessionLock {
    pub(in crate::hooks) fn acquire(dir: &Path, session: &str) -> io::Result<Self> {
        let file = Self::open(dir, session)?;
        file.lock()?;
        Ok(Self { _file: file })
    }

    pub(in crate::hooks) fn try_acquire(dir: &Path, session: &str) -> io::Result<Option<Self>> {
        let file = Self::open(dir, session)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    fn open(dir: &Path, session: &str) -> io::Result<File> {
        OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(super::lock_path_for(dir, session))
    }
}

#[cfg(test)]
mod tests;
