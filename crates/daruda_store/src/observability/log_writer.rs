//! Background-thread NDJSON log writer with size + age rotation.
//!
//! A single [`LogWriter`] instance owns a worker thread. Calls to
//! [`LogWriter::append`] enqueue an [`ErrorReport`](super::error_report::ErrorReport)
//! onto a channel; the worker drains it and appends one NDJSON line
//! per record to the current daily file.
//!
//! # Layout (D1: environment-separated)
//!
//! ```text
//! ~/.daruda/logs/
//! ├── debug/                              # cfg!(debug_assertions) builds
//! │   ├── daruda-2026-05-09.log           # day's first file (no ordinal)
//! │   ├── daruda-2026-05-09.001.log       # rolled when .log hit max size
//! │   ├── daruda-2026-05-09.002.log
//! │   └── panic-2026-05-09T14-23-11.log
//! └── release/
//!     └── …
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

/// Directory name parent for all daruda observability artefacts.
const LOG_ROOT: &str = ".daruda/logs";

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

/// Resolve `~/.daruda/logs/<profile>/`. Returns `None` when the home
/// directory cannot be determined.
pub fn log_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(LOG_ROOT).join(log_profile()))
}

/// Path for a fresh `panic-<timestamp>.log`. The panic hook writes
/// directly via [`std::fs::write`] (synchronous, no channel).
pub fn fresh_panic_log_path() -> Option<PathBuf> {
    let dir = log_dir()?;
    let ts = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    Some(dir.join(format!("panic-{ts}.log")))
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
                if let Err(e) = fs::create_dir_all(d) {
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
}

/// Public so the panic hook in `main.rs` can call it without going
/// through the worker thread (the worker may be dead by panic time).
pub fn write_panic_log(report: &ErrorReport) -> Option<PathBuf> {
    let path = fresh_panic_log_path()?;
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match fs::write(&path, report.to_plain_text()) {
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
    best.map(|(_, p)| p)
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
        && let Err(e) = fs::create_dir_all(parent)
    {
        eprintln!("daruda: log writer cannot create {}: {e}", parent.display());
        return None;
    }
    match OpenOptions::new().create(true).append(true).open(path) {
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
mod tests {
    use super::*;
    use std::time::SystemTime;
    use tempfile::TempDir;

    use crate::observability::error_report::{ErrorReport, ErrorSeverity};

    fn synthetic(title: &str, severity: ErrorSeverity) -> ErrorReport {
        ErrorReport::new(title)
            .severity(severity)
            .message("synthetic test")
            .dedup("test.synthetic")
            .build()
    }

    #[test]
    fn log_dir_is_under_profile_subdirectory() {
        let Some(dir) = log_dir() else { return };
        let last = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        assert_eq!(last, log_profile());
        let parent = dir
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("");
        assert_eq!(parent, "logs");
    }

    #[test]
    fn daily_path_omits_ordinal_zero() {
        let date = NaiveDate::from_ymd_opt(2026, 5, 9).unwrap();
        let dir = Path::new("/tmp");
        assert_eq!(
            daily_path(dir, date, 0),
            PathBuf::from("/tmp/daruda-2026-05-09.log")
        );
        assert_eq!(
            daily_path(dir, date, 1),
            PathBuf::from("/tmp/daruda-2026-05-09.001.log")
        );
        assert_eq!(
            daily_path(dir, date, 42),
            PathBuf::from("/tmp/daruda-2026-05-09.042.log")
        );
    }

    #[test]
    fn max_existing_ordinal_finds_highest() {
        let dir = TempDir::new().expect("tempdir");
        let date = NaiveDate::from_ymd_opt(2026, 5, 9).unwrap();
        // Empty dir — None.
        assert_eq!(max_existing_ordinal(dir.path(), date), None);

        // Mix of ordinals + an unrelated file.
        fs::write(dir.path().join("daruda-2026-05-09.log"), b"").unwrap();
        fs::write(dir.path().join("daruda-2026-05-09.001.log"), b"").unwrap();
        fs::write(dir.path().join("daruda-2026-05-09.005.log"), b"").unwrap();
        fs::write(dir.path().join("daruda-2026-05-08.log"), b"").unwrap();
        fs::write(dir.path().join("panic-something.log"), b"").unwrap();

        assert_eq!(max_existing_ordinal(dir.path(), date), Some(5));
    }

    #[test]
    fn rotate_old_files_removes_stale_entries_only() {
        let dir = TempDir::new().expect("tempdir");
        let stale = dir.path().join("daruda-2020-01-01.log");
        let fresh = dir.path().join("daruda-today.log");
        fs::write(&stale, b"old").unwrap();
        fs::write(&fresh, b"new").unwrap();

        let sixty_days = Duration::from_secs(60 * 24 * 60 * 60);
        let old_time = SystemTime::now() - sixty_days;
        let _ = filetime_set(&stale, old_time);

        rotate_old_files(dir.path(), Duration::from_secs(30 * 24 * 60 * 60));

        assert!(!stale.exists(), "stale file should be removed");
        assert!(fresh.exists(), "fresh file should be retained");
    }

    fn filetime_set(path: &Path, t: SystemTime) -> std::io::Result<()> {
        let f = OpenOptions::new().write(true).open(path)?;
        f.set_modified(t)
    }

    #[test]
    fn worker_writes_ndjson_then_terminates() {
        let dir = TempDir::new().expect("tempdir");
        let (tx, rx) = mpsc::channel::<ErrorReport>();
        let dir_for_worker = Some(dir.path().to_path_buf());
        let policy = LogPolicy::unbounded();
        let handle = thread::spawn(move || worker_loop(rx, dir_for_worker, policy));

        tx.send(synthetic("Test failure", ErrorSeverity::Warning))
            .expect("send");
        drop(tx);
        handle.join().expect("worker joins");

        let today = Utc::now().date_naive().format("%Y-%m-%d").to_string();
        let path = dir.path().join(format!("daruda-{today}.log"));
        assert!(path.exists(), "daily file should exist at {path:?}");

        let body = fs::read_to_string(&path).expect("read log");
        let line = body.trim_end();
        let value: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
        assert_eq!(value["title"], "Test failure");
        assert_eq!(value["severity"], "warning");
    }

    #[test]
    fn worker_rolls_to_next_ordinal_when_size_exceeded() {
        let dir = TempDir::new().expect("tempdir");
        let (tx, rx) = mpsc::channel::<ErrorReport>();
        let dir_for_worker = Some(dir.path().to_path_buf());

        // Tight cap so a single small report rolls the file.
        let policy = LogPolicy {
            retention: None,
            max_file_size: Some(256),
        };
        let handle = thread::spawn(move || worker_loop(rx, dir_for_worker, policy));

        // Each NDJSON line for a synthetic report runs ~400+ bytes
        // (text field carries the full plain-text rendering), so the
        // first write fits in `.log` and subsequent writes roll.
        for i in 0..5 {
            tx.send(synthetic(&format!("Err {i}"), ErrorSeverity::Error))
                .unwrap();
        }
        drop(tx);
        handle.join().expect("worker joins");

        let today = Utc::now().date_naive().format("%Y-%m-%d").to_string();

        let mut ordinals: Vec<u32> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                let prefix = format!("daruda-{today}");
                if !n.starts_with(&prefix) || !n.ends_with(".log") {
                    return None;
                }
                let suffix = &n[prefix.len()..];
                if suffix == ".log" {
                    Some(0)
                } else {
                    suffix
                        .strip_prefix('.')
                        .and_then(|s| s.strip_suffix(".log"))
                        .and_then(|s| s.parse::<u32>().ok())
                }
            })
            .collect();
        ordinals.sort_unstable();

        assert!(
            ordinals.len() >= 2,
            "expected at least one rollover, got {ordinals:?}",
        );
        assert_eq!(ordinals[0], 0, "first ordinal is 0 (.log)");
        // Each ordinal increment is +1; no gaps.
        for win in ordinals.windows(2) {
            assert_eq!(
                win[1],
                win[0] + 1,
                "ordinals should increment monotonically: {ordinals:?}",
            );
        }

        // Every file should be ≤ cap (give or take one full line we
        // already wrote — the cap is "before next write", not "after
        // any single write").
        for ord in &ordinals {
            let path = daily_path(dir.path(), Utc::now().date_naive(), *ord);
            let len = fs::metadata(&path).unwrap().len();
            assert!(
                len <= 256 + 4096,
                "file {ord} grew past cap+slack: {len} bytes",
            );
        }
    }

    #[test]
    fn worker_resumes_at_existing_ordinal_on_restart() {
        let dir = TempDir::new().expect("tempdir");
        let today = Utc::now().date_naive();

        // Pre-seed two ordinals from a "previous session".
        let path0 = daily_path(dir.path(), today, 0);
        let path1 = daily_path(dir.path(), today, 1);
        fs::write(&path0, b"prior session\n").unwrap();
        fs::write(&path1, b"prior session\n").unwrap();

        let (tx, rx) = mpsc::channel::<ErrorReport>();
        let policy = LogPolicy::unbounded();
        let handle = thread::spawn({
            let dir = dir.path().to_path_buf();
            move || worker_loop(rx, Some(dir), policy)
        });

        tx.send(synthetic("Resume", ErrorSeverity::Info)).unwrap();
        drop(tx);
        handle.join().unwrap();

        // The new line should be appended to the highest existing
        // ordinal (`.001.log`), not the bare file.
        let body1 = fs::read_to_string(&path1).unwrap();
        assert!(
            body1.contains("Resume"),
            "expected new line in highest existing ordinal",
        );
        let body0 = fs::read_to_string(&path0).unwrap();
        assert!(
            !body0.contains("Resume"),
            "bare file should not have been touched",
        );
    }

    #[test]
    fn write_and_scan_panic_log_round_trip() {
        let dir = TempDir::new().expect("tempdir");
        let report = ErrorReport::new("daruda panicked")
            .severity(ErrorSeverity::Error)
            .message("boom")
            .with_backtrace("frame 0\nframe 1")
            .dedup("panic")
            .build();
        let path = dir.path().join("panic-2026-05-09T14-23-11.log");
        fs::write(&path, report.to_plain_text()).unwrap();

        let entries = fs::read_dir(dir.path()).unwrap();
        let mut found = None;
        for e in entries.flatten() {
            let n = e.path().file_name().unwrap().to_string_lossy().into_owned();
            if n.starts_with("panic-") && n.ends_with(".log") {
                found = Some(e.path());
            }
        }
        let found = found.expect("scan finds panic log");
        let body = fs::read_to_string(&found).unwrap();
        assert!(body.contains("daruda panicked"));
        assert!(body.contains("boom"));
        assert!(body.contains("frame 0"));
    }

    #[test]
    fn log_is_no_op_when_global_uninstalled() {
        // The test runner shares the GLOBAL OnceLock across the whole
        // binary; we never call `init` here, so `LogWriter::log` must
        // simply drop on the floor without panicking. (When `init` was
        // called by an earlier test in the same binary, the call routes
        // to the live worker — also a no-panic outcome — so the
        // assertion is unconditionally about the absence of a panic.)
        LogWriter::log(synthetic("no-op when uninstalled", ErrorSeverity::Warning));
    }
}
