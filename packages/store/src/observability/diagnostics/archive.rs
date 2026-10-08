//! Atomic, non-overwriting ZIP persistence of already-filtered diagnostics.

use chrono::{DateTime, Utc};
use serde_json::json;
use std::{io::Write as _, path::Path};
use zip::{ZipWriter, write::SimpleFileOptions};

pub(super) fn save(
    output: &Path,
    now: DateTime<Utc>,
    events: Vec<String>,
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
            "schema": 1, "system": super::super::system_info::summary(), "created_at": now,
            "privacy": "Raw logs, message text, context, environment and transcripts are excluded.",
        }),
    )?;
    archive.start_file("events.ndjson", options)?;
    for event in events {
        archive.write_all(event.as_bytes())?;
        archive.write_all(b"\n")?;
    }
    archive.finish()?.sync_all()?;
    temporary.persist_noclobber(output)?;
    Ok(())
}
