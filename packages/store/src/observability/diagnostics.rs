//! Explicit local export of bounded, structured diagnostics, without payloads.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs::File;
use std::io::{BufRead as _, BufReader, Read as _, Seek as _, SeekFrom, Write as _};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::json;
use zip::{ZipWriter, write::SimpleFileOptions};

use super::error_report::ErrorReport;

const FILE_LIMIT: u64 = 10 * 1024 * 1024;
const EVENT_LIMIT: usize = 10_000;
const FILE_COUNT: usize = 12;
const LOOKBACK_DAYS: i64 = 3;

/// Export only timestamps, severity, source locations, and known error codes.
/// Message bodies, context, transcripts, credentials and raw wire logs never
/// enter the archive. Persist atomically and never overwrite an existing file.
pub fn export(
    logs: &Path,
    output: &Path,
    now: DateTime<Utc>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let parent = output
        .parent()
        .ok_or("diagnostic output needs a parent directory")?;
    let temporary = tempfile::NamedTempFile::new_in(parent)?;
    let mut archive = ZipWriter::new(temporary.reopen()?);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    archive.start_file("system.json", options)?;
    serde_json::to_writer_pretty(
        &mut archive,
        &json!({
            "schema": 1, "system": super::system_info::summary(), "created_at": now,
            "privacy": "Raw logs, message text, context, environment and transcripts are excluded.",
        }),
    )?;
    archive.start_file("events.ndjson", options)?;
    for event in recent_events(logs, now)? {
        archive.write_all(event.as_bytes())?;
        archive.write_all(b"\n")?;
    }
    archive.finish()?.sync_all()?;
    temporary.persist_noclobber(output)?;
    Ok(())
}

fn log_order(name: &str) -> Option<(chrono::NaiveDate, u32)> {
    let stem = name.strip_prefix("daruda-")?.strip_suffix(".log")?;
    let (date, ordinal) = stem.split_once('.').unwrap_or((stem, "0"));
    Some((
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?,
        ordinal.parse().ok()?,
    ))
}

fn recent_events(logs: &Path, now: DateTime<Utc>) -> std::io::Result<Vec<String>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(logs)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && let Some(order) = entry.file_name().to_str().and_then(log_order)
        {
            files.push((order, entry.path()));
        }
    }
    files.sort_by_key(|(order, _)| Reverse(*order));
    let cutoff = now - chrono::Duration::days(LOOKBACK_DAYS);
    // Retain only projected events: raw payloads never accumulate in memory.
    let mut events = BinaryHeap::new();
    let mut sequence = 0_u64;
    for (_, path) in files.into_iter().take(FILE_COUNT) {
        let mut file = File::open(path)?;
        let start = file.metadata()?.len().saturating_sub(FILE_LIMIT);
        file.seek(SeekFrom::Start(start))?;
        let mut reader = BufReader::new(file.take(FILE_LIMIT));
        if start > 0 {
            // The bounded tail may start in the middle of an NDJSON record.
            reader.read_until(b'\n', &mut Vec::new())?;
        }
        for line in reader.lines() {
            let line = line?;
            let Ok(report) = serde_json::from_str::<ErrorReport>(&line) else {
                continue;
            };
            if report.timestamp < cutoff || report.timestamp > now {
                continue;
            }
            sequence += 1;
            events.push(Reverse((
                report.timestamp,
                sequence,
                public_event(&report).to_string(),
            )));
            if events.len() > EVENT_LIMIT {
                events.pop();
            }
        }
    }
    Ok(events
        .into_sorted_vec()
        .into_iter()
        .map(|Reverse((_, _, event))| event)
        .collect())
}

fn public_event(report: &ErrorReport) -> serde_json::Value {
    let location = report.location.as_deref().filter(|location| {
        (location.starts_with("packages/") || location.starts_with("packages\\"))
            && location.len() < 512
            && !location.contains("..")
            && location.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '/' | '\\' | '_' | '-' | '.' | ':')
            })
    });
    let known: Vec<_> = [
        "ENOENT",
        "EACCES",
        "EPERM",
        "ETIMEDOUT",
        "ECONNRESET",
        "os error 5",
        "os error 32",
        "os error 33",
        "os error 206",
    ]
    .into_iter()
    .filter(|code| {
        report.message.contains(code)
            || report
                .source_chain
                .iter()
                .any(|source| source.contains(code))
            || report
                .context
                .get("detail")
                .is_some_and(|detail| detail.contains(code))
    })
    .collect();
    json!({"timestamp": report.timestamp, "severity": report.severity, "location": location, "error_codes": known})
}

#[cfg(test)]
mod tests;
