//! Explicit local export of bounded, structured diagnostics, without payloads.

use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

mod archive;
mod filter;
mod selection;

const FILE_LIMIT: u64 = 10 * 1024 * 1024;
const EVENT_LIMIT: usize = 10_000;
const FILE_COUNT: usize = 12;
const LOOKBACK_DAYS: i64 = 3;

/// Export diagnostics from this process's storage namespace. Older default
/// logs remain readable, but isolated runs never inspect the user's logs.
pub fn export_current(
    output: &Path,
    now: DateTime<Utc>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    LogRepository::current().export(output, now)
}

struct LogRepository {
    directories: Vec<LogSource>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LogSource {
    Current(PathBuf),
    Compatibility(PathBuf),
}

impl LogRepository {
    fn current() -> Self {
        Self {
            directories: crate::storage::StorageLayout::current().diagnostic_sources(),
        }
    }

    fn export(
        &self,
        output: &Path,
        now: DateTime<Utc>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.directories.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Cannot resolve diagnostic storage directories",
            )
            .into());
        }
        export_from(&self.directories, output, now)
    }
}

/// Export only timestamps, severity, source locations, and known error codes.
/// Message bodies, context, transcripts, credentials and raw wire logs never
/// enter the archive. Persist atomically and never overwrite an existing file.
pub fn export(
    logs: &Path,
    output: &Path,
    now: DateTime<Utc>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    LogRepository {
        directories: vec![LogSource::Current(logs.to_path_buf())],
    }
    .export(output, now)
}

fn export_from(
    sources: &[LogSource],
    output: &Path,
    now: DateTime<Utc>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let files = selection::files(sources)?;
    let events = filter::recent_events(&files, now)?;
    archive::save(output, now, events)
}

#[cfg(test)]
mod tests;
