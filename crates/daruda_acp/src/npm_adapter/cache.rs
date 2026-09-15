//! Per-package coordination and eviction of installations that have no users.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::preparation::{PreparationContext, PreparationError, PreparationKind};

const LOCK_TIMEOUT: Duration = Duration::from_secs(180);
const LOCK_POLL: Duration = Duration::from_millis(25);
const KEEP_INSTALLATIONS: usize = 3;
const KEEP_INVALID: usize = 1;
const LEASE: &str = ".lease";
const LAST_USED: &str = ".last-used";
pub(super) const VERSION_PREFIX: &str = "version-";
const INVALID_PREFIX: &str = ".invalid-";

#[derive(Clone, Debug)]
pub(crate) struct InstallationLease {
    _file: Arc<FileLock>,
}

#[derive(Debug)]
pub(super) struct FileLock(File);

impl Drop for FileLock {
    fn drop(&mut self) {
        // Unlock at the ownership boundary, not after a forked child closes
        // its inherited descriptor. Closing this File remains the fallback.
        let _ = self.0.unlock();
    }
}

impl InstallationLease {
    pub(super) fn acquire(directory: &Path) -> Result<Self, PreparationError> {
        let file = lock_file(&directory.join(LEASE))?;
        file.lock_shared()?;
        let file = FileLock(file);
        fs::write(directory.join(LAST_USED), [])?;
        Ok(Self {
            _file: Arc::new(file),
        })
    }
}

pub(super) fn lock_file(path: &Path) -> Result<File, PreparationError> {
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?)
}

pub(super) fn lock_package(
    directory: &Path,
    context: &PreparationContext<'_>,
) -> Result<FileLock, PreparationError> {
    fs::create_dir_all(directory)?;
    let file = lock_file(&directory.join(".install.lock"))?;
    let started = Instant::now();
    loop {
        context.check()?;
        match file.try_lock() {
            Ok(()) => return Ok(FileLock(file)),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        if started.elapsed() >= LOCK_TIMEOUT {
            return Err(PreparationError::new(
                PreparationKind::Timeout,
                "waiting for adapter installation lock timed out",
            ));
        }
        std::thread::sleep(LOCK_POLL);
    }
}

/// Held by the package installer while replacing a damaged version.
pub(super) fn exclusive_lease(directory: &Path) -> Result<Option<FileLock>, PreparationError> {
    let file = lock_file(&directory.join(LEASE))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(FileLock(file))),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error.into()),
    }
}

pub(super) fn write_receipt(path: &Path, version: &str) -> Result<(), PreparationError> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().expect("receipt parent"))?;
    file.write_all(version.as_bytes())?;
    file.persist(path)
        .map_err(|error| PreparationError::from(error.error))?;
    Ok(())
}

/// The caller holds the package lock, so no new lease can race this sweep.
pub(super) fn sweep(
    directory: &Path,
    context: &PreparationContext<'_>,
) -> Result<(), PreparationError> {
    sweep_prefix(directory, VERSION_PREFIX, KEEP_INSTALLATIONS, context)?;
    sweep_prefix(directory, INVALID_PREFIX, KEEP_INVALID, context)
}

fn sweep_prefix(
    directory: &Path,
    prefix: &str,
    keep: usize,
    context: &PreparationContext<'_>,
) -> Result<(), PreparationError> {
    let mut entries: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in fs::read_dir(directory)? {
        context.check()?;
        let entry = entry?;
        if !entry.file_type()?.is_dir() || !entry.file_name().to_string_lossy().starts_with(prefix)
        {
            continue;
        }
        let path = entry.path();
        let modified = match fs::metadata(path.join(LAST_USED)) {
            Ok(metadata) => metadata.modified()?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                entry.metadata()?.modified()?
            }
            Err(error) => return Err(error.into()),
        };
        entries.push((modified, path));
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    for (_, path) in entries.into_iter().skip(keep) {
        context.check()?;
        if let Some(_lease) = exclusive_lease(&path)? {
            // Only validated, direct children of this package's cache are eligible.
            fs::remove_dir_all(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
