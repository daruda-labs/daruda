//! Append-only NDJSON file sink for mark records.
//!
//! [`NdjsonFileSink`] wraps a `BufWriter<File>` opened in append mode.
//! Each call to [`NdjsonFileSink::append_line`] writes one NDJSON line
//! (the caller provides the already-serialized string; no trailing `\n`
//! required — the sink appends it).
//!
//! # Size cap
//!
//! A single advisory warning is emitted via [`LogWriter::log`] when the
//! cumulative bytes written exceed [`MARKS_FILE_SIZE_CAP`]. Writes
//! continue beyond the cap — compaction lands in a later sprint.
//!
//! # fsync
//!
//! The sink calls `sync_data` on the underlying file at most once per
//! [`FSYNC_PERIOD`]. Mutations happen at human interaction speed, so
//! this bounds data loss on power failure to a few records without
//! making `sync_data` a hot path.
//!
//! # Partial-write safety
//!
//! I/O errors from [`write_all`](std::io::Write::write_all) are
//! returned to the caller verbatim. The NDJSON replay logic in
//! `daruda_terminal::session::interval_tree::persistence` already
//! handles a truncated last line as `skipped_partial`, so callers do
//! not need to attempt recovery here.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::observability::error_report::{ErrorReport, ErrorSeverity};
use crate::observability::log_writer::LogWriter;

/// Advisory file size cap: 100 MiB. A single warning is emitted when
/// bytes written exceed this value; appends continue (compaction is a
/// later task).
pub const MARKS_FILE_SIZE_CAP: u64 = 100 * 1024 * 1024;

/// How often `sync_data` is called on the underlying file. 5 seconds
/// bounds data loss on power failure to a few records without making
/// `sync_data` a hot path for this low-rate append-only writer.
const FSYNC_PERIOD: Duration = Duration::from_secs(5);

/// File name used inside the lane directory.
const MARKS_FILE_NAME: &str = "marks.ndjson";

/// Append-only NDJSON sink that writes serialized mark records to
/// `<lane_dir>/marks.ndjson`.
///
/// Construct with [`NdjsonFileSink::open`] (production) or
/// [`NdjsonFileSink::open_with_cap`] (tests, to exercise the cap
/// warning with a smaller threshold).
pub struct NdjsonFileSink {
    writer: BufWriter<File>,
    path: PathBuf,
    bytes_written: u64,
    cap: u64,
    cap_warned: bool,
    last_fsync: Instant,
}

impl NdjsonFileSink {
    /// Open (or create) `<lane_dir>/marks.ndjson` in append mode.
    ///
    /// `lane_dir` is created if it does not already exist. The
    /// returned sink initialises `bytes_written` from the current
    /// file length so the cap accounting is accurate across restarts.
    pub fn open(lane_dir: &Path) -> io::Result<Self> {
        Self::open_with_cap(lane_dir, MARKS_FILE_SIZE_CAP)
    }

    /// Like [`open`](Self::open) but with a caller-supplied cap.
    ///
    /// Prefer [`open`](Self::open) at production call sites. This
    /// variant exists so tests can exercise the cap-warning path
    /// without writing 100 MiB of data.
    pub(crate) fn open_with_cap(lane_dir: &Path, cap: u64) -> io::Result<Self> {
        fs::create_dir_all(lane_dir)?;
        let path = lane_dir.join(MARKS_FILE_NAME);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let bytes_written = file.metadata().map(|m| m.len()).unwrap_or(0);
        let writer = BufWriter::new(file);
        Ok(Self {
            writer,
            path,
            bytes_written,
            cap,
            cap_warned: false,
            last_fsync: Instant::now(),
        })
    }

    /// Append a single NDJSON line. `line` must not contain a trailing
    /// newline — the sink appends `\n` itself. Returns the number of
    /// bytes written (including the newline).
    ///
    /// On I/O failure the error is returned to the caller; no bytes
    /// are considered written for accounting purposes. The caller is
    /// responsible for routing the error to [`LogWriter::log`].
    pub fn append_line(&mut self, line: &str) -> io::Result<usize> {
        let bytes = line.as_bytes();
        self.writer.write_all(bytes)?;
        self.writer.write_all(b"\n")?;
        let written = bytes.len() + 1;
        self.bytes_written = self.bytes_written.saturating_add(written as u64);

        self.maybe_warn_cap();
        self.maybe_fsync()?;

        Ok(written)
    }

    /// Cumulative bytes written through this sink instance (including
    /// bytes that existed in the file before the sink was opened).
    #[cfg(test)]
    pub(crate) fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Whether the cap warning has already been emitted.
    #[cfg(test)]
    pub(crate) fn is_cap_warned(&self) -> bool {
        self.cap_warned
    }

    /// Path of the underlying marks file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    // ---- private helpers ----

    fn maybe_warn_cap(&mut self) {
        if self.cap_warned || self.bytes_written <= self.cap {
            return;
        }
        let report = ErrorReport::new("marks.ndjson file size cap exceeded")
            .severity(ErrorSeverity::Warning)
            .message(format!(
                "marks.ndjson has grown to {} bytes (cap: {} bytes). \
                 Compaction has not landed yet — writes continue. \
                 File: {}",
                self.bytes_written,
                self.cap,
                self.path.display(),
            ))
            .with_context("subsystem", "marks.ndjson_sink")
            .with_context("bytes_written", self.bytes_written.to_string())
            .with_context("cap_bytes", self.cap.to_string())
            .dedup("marks.ndjson_sink.cap_exceeded")
            .build();
        LogWriter::log(report);
        self.cap_warned = true;
    }

    fn maybe_fsync(&mut self) -> io::Result<()> {
        if self.last_fsync.elapsed() < FSYNC_PERIOD {
            return Ok(());
        }
        // Flush BufWriter first so the kernel has the bytes.
        self.writer.flush()?;
        self.writer.get_ref().sync_data()?;
        self.last_fsync = Instant::now();
        Ok(())
    }
}

impl Drop for NdjsonFileSink {
    fn drop(&mut self) {
        // Best-effort flush on drop; errors are swallowed because
        // panicking in Drop is worse than a partial flush.
        let _ = self.writer.flush();
    }
}

/// Open the marks file at `path` and return an iterator over its
/// lines. Each item is `io::Result<String>` — the natural shape of
/// `BufReader::lines()` and what `replay()` expects.
///
/// Returns an `io::Error` with `kind() == ErrorKind::NotFound` when
/// the file does not exist so callers can treat a missing file as a
/// fresh tree.
pub fn replay_iter(path: &Path) -> io::Result<io::Lines<io::BufReader<File>>> {
    use std::io::BufRead;
    let file = File::open(path)?;
    Ok(io::BufReader::new(file).lines())
}

/// Convenience: return the canonical path for a lane's marks file
/// given the lane directory.
pub fn marks_path(lane_dir: &Path) -> PathBuf {
    lane_dir.join(MARKS_FILE_NAME)
}
