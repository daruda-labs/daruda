//! Background-thread NDJSON log writer with size + age rotation.
//!
//! A single [`LogWriter`] instance owns a worker thread. Calls to
//! [`LogWriter::append`] enqueue an [`ErrorReport`](super::error_report::ErrorReport)
//! onto a channel; the worker drains it and appends one NDJSON line
//! per record to the current daily file.
//!
//! # Layout
//!
//! ```text
//! Windows: %LOCALAPPDATA%/daruda/logs/<profile>/
//! macOS:   ~/Library/Logs/daruda/<profile>/
//! Linux:   $XDG_STATE_HOME/daruda/logs/<profile>/
//! Override: $DARUDA_DATA_DIR/logs/
//! ```
//!
//! # Rolling rules
//!
//! Driven by a [`LogPolicy`] passed to [`LogWriter::init`]:
//!
//! - **Date rollover** — every UTC midnight the writer switches to a
//!   fresh `daruda-<today>.log`. Always on; not tunable.
//! - **Size cap** — when the active file reaches `policy.max_file_size`
//!   bytes the writer closes it and opens
//!   `daruda-<today>.NNN.log`, where `NNN` is the next 3-digit ordinal
//!   (zero-padded). `None` disables size capping.
//! - **Retention** — files older than `policy.retention` are pruned at
//!   startup and once per 24 h. `None` keeps every file forever.
//!
//! # Lifecycle
//!
//! [`LogWriter::init`] is idempotent — repeated calls return the
//! existing instance. The worker thread holds the only consumer end
//! of the channel and exits when the producer side is dropped.
//!
//! # Failure handling
//!
//! Filesystem failures during init / append fall back to `eprintln!`
//! once and then go silent. The observability layer must never panic
//! the parent process.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};
use std::thread;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, NaiveDate, Utc};

use super::error_report::ErrorReport;

static GLOBAL: OnceLock<LogWriter> = OnceLock::new();

/// Retention + size policy passed to [`LogWriter::init`]. Both knobs
/// are optional — `None` disables that rule.
#[derive(Clone, Copy, Debug)]
pub struct LogPolicy {
    /// Files older than this are pruned. `None` = keep forever.
    pub retention: Option<Duration>,
    /// Per-file size cap. The active file rolls to the next ordinal
    /// once it reaches this size. `None` = no size cap.
    pub max_file_size: Option<u64>,
}

impl LogPolicy {
    /// Conservative default: 30-day retention, 10 MB per-file cap.
    /// Mirrors `daruda_config::LogsConfig::default` so a daruda binary
    /// that forgets to read the config still gets sane behaviour.
    pub const fn defaults() -> Self {
        Self {
            retention: Some(Duration::from_secs(30 * 24 * 60 * 60)),
            max_file_size: Some(10 * 1024 * 1024),
        }
    }

    /// Disable both rules. Useful in tests that drive the writer
    /// directly and don't want age / size machinery interfering.
    pub const fn unbounded() -> Self {
        Self {
            retention: None,
            max_file_size: None,
        }
    }
}

impl Default for LogPolicy {
    fn default() -> Self {
        Self::defaults()
    }
}

/// Build/runtime profile bucket — `debug` for `cfg!(debug_assertions)`
/// builds, `release` otherwise, unless overridden by `DARUDA_PROFILE`.
/// Mirrors `crate::profile::active_profile()` so data and logs share
/// a single source of truth.
pub fn log_profile() -> &'static str {
    crate::profile::active_profile()
}

/// Native profile log directory, or `<DARUDA_DATA_DIR>/logs` when overridden.
/// Returns `None` when the OS cannot resolve the native directory.
pub fn log_dir() -> Option<PathBuf> {
    crate::storage::StorageLayout::current().logs()
}

/// Path for a fresh `panic-<timestamp>.log`. The panic hook writes
/// directly via [`std::fs::write`] (synchronous, no channel).
pub fn fresh_panic_log_path() -> Option<PathBuf> {
    let dir = log_dir()?;
    let ts = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    fresh_panic_log_path_in(&dir, &ts)
}

/// Pick a panic-log path under `dir` for timestamp `ts` that does not
/// collide with an existing file. The timestamp has second resolution,
/// but one crash can invoke the panic hook twice within a second (an
/// unwinding panic that crosses a nounwind frame raises a second
/// "panic in a function that cannot unwind") — a bare name would
/// overwrite the first report, losing the original panic message.
fn fresh_panic_log_path_in(dir: &Path, ts: &str) -> Option<PathBuf> {
    let base = dir.join(format!("panic-{ts}.log"));
    if !base.exists() {
        return Some(base);
    }
    (1..100)
        .map(|n| dir.join(format!("panic-{ts}.{n:02}.log")))
        .find(|p| !p.exists())
}

/// Path of the day's primary daily file (ordinal 0). Independent of
/// any size-rolled siblings — `[Open log file]` in the Details modal
/// uses this so the user always lands at the canonical daily file
/// even when later ordinals exist for that day.
pub fn today_log_path() -> Option<PathBuf> {
    let dir = log_dir()?;
    let date = Utc::now().date_naive().format("%Y-%m-%d").to_string();
    Some(dir.join(format!("daruda-{date}.log")))
}

/// Build the path for a given date + ordinal. Ordinal `0` produces the
/// bare `daruda-YYYY-MM-DD.log`; `1+` gets the zero-padded
/// `daruda-YYYY-MM-DD.NNN.log` form.
fn daily_path(dir: &Path, date: NaiveDate, ordinal: u32) -> PathBuf {
    let date_str = date.format("%Y-%m-%d").to_string();
    if ordinal == 0 {
        dir.join(format!("daruda-{date_str}.log"))
    } else {
        dir.join(format!("daruda-{date_str}.{ordinal:03}.log"))
    }
}

/// Inspect `dir` and return the highest ordinal already used for
/// `date`. `None` when no file for that date exists.
fn max_existing_ordinal(dir: &Path, date: NaiveDate) -> Option<u32> {
    let date_prefix = format!("daruda-{}", date.format("%Y-%m-%d"));
    let entries = fs::read_dir(dir).ok()?;
    let mut best: Option<u32> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(&date_prefix) || !name.ends_with(".log") {
            continue;
        }
        // `daruda-YYYY-MM-DD.log`              → ordinal 0
        // `daruda-YYYY-MM-DD.001.log`          → ordinal 1
        let suffix = &name[date_prefix.len()..];
        let ord = if suffix == ".log" {
            0
        } else if let Some(num_str) = suffix
            .strip_prefix('.')
            .and_then(|s| s.strip_suffix(".log"))
            && let Ok(n) = num_str.parse::<u32>()
        {
            n
        } else {
            continue;
        };
        best = Some(best.map_or(ord, |b| b.max(ord)));
    }
    best
}

/// Background-thread NDJSON writer. Construct via [`LogWriter::init`]
/// from `main()`; later code reaches it via [`LogWriter::global`].
pub struct LogWriter {
    sender: mpsc::Sender<ErrorReport>,
    /// Kept so future tests can take ownership and assert termination.
    /// The thread runs for the lifetime of the process and is reaped
    /// by the OS at exit.
    _thread: Option<thread::JoinHandle<()>>,
}

impl LogWriter {
    /// Spawn the worker thread and install the global singleton.
    /// Idempotent — only the first call has effect.
    ///
    /// Returns a reference to the live instance regardless of whether
    /// this call performed the init. When `log_dir()` cannot be
    /// resolved the writer is still installed but every append is a
    /// silent no-op.
    pub fn init(policy: LogPolicy) -> &'static LogWriter {
        GLOBAL.get_or_init(|| {
            let dir = log_dir();
            if let Some(d) = &dir {
                if let Err(e) = daruda_core::path::create_owner_only_dir(d) {
                    eprintln!(
                        "daruda: log writer disabled — could not create {}: {e}",
                        d.display()
                    );
                }
                if let Some(retention) = policy.retention {
                    rotate_old_files(d, retention);
                }
            }

            let (tx, rx) = mpsc::channel::<ErrorReport>();
            let dir_for_thread = dir;
            let thread = thread::Builder::new()
                .name("daruda-log-writer".to_string())
                .spawn(move || worker_loop(rx, dir_for_thread, policy))
                .ok();

            LogWriter {
                sender: tx,
                _thread: thread,
            }
        })
    }

    /// Reference to the installed singleton, if any.
    pub fn global() -> Option<&'static LogWriter> {
        GLOBAL.get()
    }

    /// Enqueue a report. Drop on disconnected channel — observability
    /// is best-effort by design.
    pub fn append(&self, report: ErrorReport) {
        let _ = self.sender.send(report);
    }

    /// Convenience: append `report` via the installed singleton, or
    /// silently drop if no global writer is installed yet (early
    /// startup, tests). Use from sites that have no `Workspace` to
    /// route the report through `report_error` — the on-disk NDJSON
    /// log is the only surviving surface there.
    pub fn log(report: ErrorReport) {
        if let Some(w) = Self::global() {
            w.append(report);
        }
    }

    /// Persist a terminal failure before process exit, without the worker queue.
    /// Returns the daily log path only after the report has been synced.
    pub fn log_sync(report: &ErrorReport) -> std::io::Result<PathBuf> {
        let dir = log_dir().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Cannot resolve log storage")
        })?;
        append_sync_in(&dir, report).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!(
                    "Cannot persist terminal failure in {}: {error}",
                    dir.display()
                ),
            )
        })
    }
}

fn append_sync_in(dir: &Path, report: &ErrorReport) -> std::io::Result<PathBuf> {
    daruda_core::path::create_owner_only_dir(dir)?;
    let date = Utc::now().date_naive();
    let ordinal = max_existing_ordinal(dir, date).unwrap_or(0);
    let path = daily_path(dir, date, ordinal);
    // Terminal failures bypass rolling: one final record must survive exit.
    let mut file =
        daruda_core::path::open_owner_only(OpenOptions::new().create(true).append(true), &path)?;
    file.write_all(report.to_ndjson_line().as_bytes())?;
    file.sync_all()?;
    Ok(path)
}

/// Public so the panic hook in `main.rs` can call it without going
/// through the worker thread (the worker may be dead by panic time).
pub fn write_panic_log(report: &ErrorReport) -> Option<PathBuf> {
    let path = fresh_panic_log_path()?;
    if let Some(parent) = path.parent() {
        let _ = daruda_core::path::create_owner_only_dir(parent);
    }
    let written = daruda_core::path::open_owner_only(
        OpenOptions::new().create(true).write(true).truncate(true),
        &path,
    )
    .and_then(|mut file| file.write_all(report.to_plain_text().as_bytes()));
    match written {
        Ok(()) => Some(path),
        Err(e) => {
            eprintln!(
                "daruda: failed to write panic log at {}: {e}",
                path.display()
            );
            None
        }
    }
}

/// Scan `<log_dir>/panic-*.log` and return the most recent path.
pub fn scan_latest_panic_log() -> Option<PathBuf> {
    let dir = log_dir()?;
    let entries = fs::read_dir(&dir).ok()?;
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name()?.to_string_lossy().into_owned();
        if !name.starts_with("panic-") || !name.ends_with(".log") {
            continue;
        }
        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        match &best {
            Some((t, _)) if *t >= mtime => {}
            _ => best = Some((mtime, path)),
        }
    }
    let (_, latest) = best?;
    Some(original_panic_sibling(latest))
}

/// Map an ordinal-suffixed panic log (`panic-<ts>.NN.log`, written by a
/// same-second follow-up hook invocation such as the "cannot unwind"
/// secondary panic) back to its base sibling `panic-<ts>.log` when that
/// file exists — the base file holds the ORIGINAL panic message, which
/// is the report worth surfacing. Paths without an ordinal (or whose
/// base sibling is missing) pass through unchanged.
fn original_panic_sibling(path: PathBuf) -> PathBuf {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return path;
    };
    let Some(stem) = name
        .strip_prefix("panic-")
        .and_then(|s| s.strip_suffix(".log"))
    else {
        return path;
    };
    // `<ts>.NN` → `<ts>`; anything else (no dot-ordinal) passes through.
    let Some((ts, ordinal)) = stem.rsplit_once('.') else {
        return path;
    };
    if ordinal.is_empty() || !ordinal.bytes().all(|b| b.is_ascii_digit()) {
        return path;
    }
    let base = path.with_file_name(format!("panic-{ts}.log"));
    if base.exists() { base } else { path }
}

/// Drains the `rx` channel; opens / rolls daily files; prunes stale
/// files once per day. Splitting from `LogWriter::init` makes the loop
/// itself testable.
pub(crate) fn worker_loop(
    rx: mpsc::Receiver<ErrorReport>,
    dir: Option<PathBuf>,
    policy: LogPolicy,
) {
    let mut current: Option<OpenFile> = None;
    let mut last_rotate: DateTime<Utc> = Utc::now();

    while let Ok(report) = rx.recv() {
        let Some(dir) = dir.as_deref() else {
            continue;
        };
        let today = Utc::now().date_naive();

        // Re-open when the date rolls over or when we have no file.
        if !matches!(&current, Some(c) if c.date == today) {
            let ordinal = max_existing_ordinal(dir, today).unwrap_or(0);
            current = OpenFile::open(dir, today, ordinal);
        }

        let line = report.to_ndjson_line();
        let line_bytes = line.as_bytes();

        // Roll on size if needed *before* writing — keeps the cap a
        // hard upper bound rather than a soft one.
        if let (Some(file), Some(cap)) = (current.as_ref(), policy.max_file_size)
            && file.size_after(line_bytes.len() as u64) > cap
        {
            let next = file.ordinal + 1;
            current = OpenFile::open(dir, today, next);
        }

        if let Some(file) = current.as_mut()
            && let Err(e) = file.write_all(line_bytes)
        {
            eprintln!("daruda: log append failed: {e}");
            current = None;
        }

        // Age-based prune once per 24 h. Cheap when nothing matches —
        // only stat()s the directory.
        if let Some(retention) = policy.retention {
            let now = Utc::now();
            if now.signed_duration_since(last_rotate).num_hours() >= 24 {
                rotate_old_files(dir, retention);
                last_rotate = now;
            }
        }
    }
}

/// File handle bundled with the metadata the rolling logic needs.
struct OpenFile {
    handle: File,
    date: NaiveDate,
    ordinal: u32,
    /// Bytes already written to `handle` at the time the file was
    /// opened. Updated by [`write_all`](Self::write_all). Used to
    /// decide when to roll without an extra `metadata()` syscall on
    /// every append.
    bytes_written: u64,
}

impl OpenFile {
    fn open(dir: &Path, date: NaiveDate, ordinal: u32) -> Option<Self> {
        let path = daily_path(dir, date, ordinal);
        let handle = open_append(&path)?;
        let bytes_written = handle.metadata().map(|m| m.len()).unwrap_or(0);
        Some(Self {
            handle,
            date,
            ordinal,
            bytes_written,
        })
    }

    fn size_after(&self, additional: u64) -> u64 {
        self.bytes_written.saturating_add(additional)
    }

    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.handle.write_all(bytes)?;
        self.bytes_written = self.bytes_written.saturating_add(bytes.len() as u64);
        Ok(())
    }
}

fn open_append(path: &Path) -> Option<File> {
    if let Some(parent) = path.parent()
        && let Err(e) = daruda_core::path::create_owner_only_dir(parent)
    {
        eprintln!("daruda: log writer cannot create {}: {e}", parent.display());
        return None;
    }
    match daruda_core::path::open_owner_only(OpenOptions::new().create(true).append(true), path) {
        Ok(f) => Some(f),
        Err(e) => {
            eprintln!("daruda: log writer cannot open {}: {e}", path.display());
            None
        }
    }
}

/// Delete files older than `retention` under `dir`. Best-effort —
/// errors are swallowed so a permission glitch on one file does not
/// abort the sweep.
pub fn rotate_old_files(dir: &Path, retention: Duration) {
    let now = SystemTime::now();
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let Ok(age) = now.duration_since(modified) else {
            continue;
        };
        if age > retention {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests;
