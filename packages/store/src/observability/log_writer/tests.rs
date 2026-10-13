use super::*;
use std::time::SystemTime;
use tempfile::TempDir;

use crate::observability::error_report::{ErrorReport, ErrorSeverity};

#[test]
fn synchronous_terminal_report_is_readable_without_a_worker() {
    let dir = TempDir::new().unwrap();
    let first = synthetic("first", ErrorSeverity::Error);
    let second = synthetic("second", ErrorSeverity::Error);
    let path = append_sync_in(dir.path(), &first).unwrap();
    assert_eq!(append_sync_in(dir.path(), &second).unwrap(), path);
    let contents = fs::read_to_string(path).unwrap();
    let reports: Vec<ErrorReport> = contents
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].title, "first");
    assert_eq!(reports[1].title, "second");
    let unavailable = dir.path().join("not-a-directory");
    fs::write(&unavailable, "preserve").unwrap();
    assert!(append_sync_in(&unavailable, &first).is_err());
    assert_eq!(fs::read_to_string(unavailable).unwrap(), "preserve");
}

fn synthetic(title: &str, severity: ErrorSeverity) -> ErrorReport {
    ErrorReport::new(title)
        .severity(severity)
        .message("synthetic test")
        .dedup("test.synthetic")
        .build()
}

#[test]
fn log_directory_uses_the_same_storage_snapshot_as_data() {
    assert_eq!(log_dir(), crate::storage::StorageLayout::current().logs());
    assert_eq!(log_profile(), crate::profile::active_profile());
}

#[test]
fn fresh_panic_log_path_uniquifies_same_second_collisions() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let ts = "2026-06-05T06-15-56";
    let first = fresh_panic_log_path_in(dir, ts).unwrap();
    assert_eq!(first, dir.join("panic-2026-06-05T06-15-56.log"));
    std::fs::write(&first, "original panic").unwrap();
    // Second hook invocation in the same second must not pick the
    // same path (it would truncate the original report away).
    let second = fresh_panic_log_path_in(dir, ts).unwrap();
    assert_eq!(second, dir.join("panic-2026-06-05T06-15-56.01.log"));
    std::fs::write(&second, "cannot-unwind follow-up").unwrap();
    let third = fresh_panic_log_path_in(dir, ts).unwrap();
    assert_eq!(third, dir.join("panic-2026-06-05T06-15-56.02.log"));
    assert_eq!(
        std::fs::read_to_string(&first).unwrap(),
        "original panic",
        "the first report must survive"
    );
}

#[test]
fn original_panic_sibling_prefers_base_report() {
    let tmp = TempDir::new().unwrap();
    let base = tmp.path().join("panic-2026-06-05T06-15-56.log");
    let follow = tmp.path().join("panic-2026-06-05T06-15-56.01.log");
    // Without the base on disk, the ordinal file passes through.
    assert_eq!(original_panic_sibling(follow.clone()), follow);
    std::fs::write(&base, "original").unwrap();
    // With the base present, the follow-up maps back to it.
    assert_eq!(original_panic_sibling(follow), base);
    // A plain base path is untouched.
    assert_eq!(original_panic_sibling(base.clone()), base);
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
