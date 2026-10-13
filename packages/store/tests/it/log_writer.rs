//! NDJSON log writer, rotation and panic logs.

use daruda_store::observability::log_writer::*;
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};

fn synthetic(title: &str, severity: ErrorSeverity) -> ErrorReport {
    ErrorReport::new(title)
        .severity(severity)
        .message("synthetic test")
        .dedup("test.synthetic")
        .build()
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
