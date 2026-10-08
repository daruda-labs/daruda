//! Discover recent rotated logs and read bounded tails without partial records.

use super::{FILE_COUNT, FILE_LIMIT, LogSource};
use std::{
    cmp::Reverse,
    fs::File,
    io::{BufRead as _, BufReader, Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
};

type LogOrder = (chrono::NaiveDate, u32);

pub(super) fn log_order(name: &str) -> Option<LogOrder> {
    let stem = name.strip_prefix("daruda-")?.strip_suffix(".log")?;
    let (date, ordinal) = stem.split_once('.').unwrap_or((stem, "0"));
    Some((
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?,
        ordinal.parse().ok()?,
    ))
}

pub(super) struct LogFile {
    path: PathBuf,
    compatibility: bool,
}

impl LogFile {
    pub(super) fn lines(&self) -> std::io::Result<Vec<String>> {
        match tail(&self.path).and_then(|lines| lines.collect()) {
            Ok(lines) => Ok(lines),
            Err(error) if self.compatibility => {
                report_compatibility_failure(&self.path, &error);
                Ok(Vec::new())
            }
            Err(error) => Err(error),
        }
    }
}

fn report_compatibility_failure(path: &Path, error: &std::io::Error) {
    use crate::observability::{
        error_report::{ErrorReport, ErrorSeverity},
        log_writer::LogWriter,
    };
    LogWriter::log(
        ErrorReport::new("Skipped unreadable compatibility logs")
            .severity(ErrorSeverity::Warning)
            .from_error(error)
            .with_context("path", crate::observability::system_info::redact_home(path))
            .dedup("diagnostics.compatibility.read")
            .build(),
    );
}

fn source_files(source: &Path) -> std::io::Result<Vec<(LogOrder, PathBuf)>> {
    let entries = match std::fs::read_dir(source) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && let Some(order) = entry.file_name().to_str().and_then(log_order)
        {
            files.push((order, entry.path()));
        }
    }
    Ok(files)
}

pub(super) fn files(sources: &[LogSource]) -> std::io::Result<Vec<LogFile>> {
    let mut files = Vec::new();
    for source in sources {
        let (path, compatibility) = match source {
            LogSource::Current(path) => (path, false),
            LogSource::Compatibility(path) => (path, true),
        };
        let source_files = match source_files(path) {
            Ok(files) => files,
            Err(error) if compatibility => {
                report_compatibility_failure(path, &error);
                continue;
            }
            Err(error) => return Err(error),
        };
        for (order, path) in source_files {
            files.push((
                order,
                LogFile {
                    path,
                    compatibility,
                },
            ));
        }
    }
    files.sort_by_key(|(order, _)| Reverse(*order));
    Ok(files
        .into_iter()
        .take(FILE_COUNT)
        .map(|(_, path)| path)
        .collect())
}

pub(super) fn tail(path: &Path) -> std::io::Result<impl Iterator<Item = std::io::Result<String>>> {
    let mut file = File::open(path)?;
    let start = file.metadata()?.len().saturating_sub(FILE_LIMIT);
    file.seek(SeekFrom::Start(start))?;
    let mut reader = BufReader::new(file.take(FILE_LIMIT));
    if start > 0 {
        // The bounded tail may start in the middle of an NDJSON record.
        reader.read_until(b'\n', &mut Vec::new())?;
    }
    Ok(reader.lines())
}
