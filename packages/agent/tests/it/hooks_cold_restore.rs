//! Tests for the public `daruda_agent::hooks::cold_restore` API.

use chrono::Utc;
use daruda_agent::SessionStatus;
use daruda_agent::hooks::cold_restore::*;
use daruda_agent::hooks::status_file::read;
use daruda_agent::hooks::status_file::{StatusFile, path_for, write_atomic};
use std::fs;
use std::time::{Duration, SystemTime};

fn policy() -> ColdRestorePolicy {
    ColdRestorePolicy::from_config_secs(300, 7)
}

fn fresh_file(state: SessionStatus) -> StatusFile {
    StatusFile::new_hook("s1", "/tmp/x", state, "Stop")
}

/// A status file whose recorded event `timestamp` is `age` in the
/// past — what `run` now ages by, independent of the file mtime.
fn file_aged(state: SessionStatus, age: Duration) -> StatusFile {
    let mut f = StatusFile::new_hook("s1", "/tmp/x", state, "Stop");
    f.timestamp = Utc::now() - chrono::Duration::from_std(age).unwrap();
    f
}

#[test]
fn classify_fresh_loads() {
    let now = SystemTime::now();
    let modified = now - Duration::from_secs(60);
    assert_eq!(classify(modified, now, &policy()), ColdRestoreAction::Load);
}

#[test]
fn classify_stale_resets() {
    let now = SystemTime::now();
    let modified = now - Duration::from_secs(600); // 10 min
    assert_eq!(classify(modified, now, &policy()), ColdRestoreAction::Reset);
}

#[test]
fn classify_old_deletes() {
    let now = SystemTime::now();
    let modified = now - Duration::from_secs(8 * 24 * 60 * 60); // 8 days
    assert_eq!(
        classify(modified, now, &policy()),
        ColdRestoreAction::Delete
    );
}

#[test]
fn classify_boundary_at_threshold_resets() {
    let now = SystemTime::now();
    let modified = now - Duration::from_secs(300);
    assert_eq!(classify(modified, now, &policy()), ColdRestoreAction::Reset);
}

#[test]
fn classify_boundary_at_ttl_deletes() {
    let now = SystemTime::now();
    let modified = now - Duration::from_secs(7 * 24 * 60 * 60);
    assert_eq!(
        classify(modified, now, &policy()),
        ColdRestoreAction::Delete
    );
}

#[test]
fn run_loads_fresh_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = path_for(dir.path(), "fresh");
    write_atomic(&p, &fresh_file(SessionStatus::Working)).unwrap();

    let loaded = run(dir.path(), &policy()).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].status, SessionStatus::Working);
    assert!(p.exists());
}

#[test]
fn run_missing_dir_yields_empty() {
    let dir = tempfile::TempDir::new().unwrap();
    let nope = dir.path().join("not-there");
    let loaded = run(&nope, &policy()).unwrap();
    assert!(loaded.is_empty());
}

/// Force a file's mtime to a specific value via std `File::set_modified`.
/// Returns `false` if the platform doesn't honour the change (rare on
/// modern macOS / Linux); the caller treats that as test-skip.
fn force_mtime(path: &std::path::Path, when: SystemTime) -> bool {
    let f = match fs::File::options().write(true).open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    if f.set_modified(when).is_err() {
        return false;
    }
    let read_back = fs::metadata(path).and_then(|m| m.modified()).ok();
    match read_back {
        Some(t) => {
            let drift = t
                .duration_since(when)
                .or_else(|_| when.duration_since(t))
                .unwrap_or(Duration::ZERO);
            drift < Duration::from_secs(2)
        }
        None => false,
    }
}

#[test]
fn run_resets_stale_and_keeps_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = path_for(dir.path(), "stale");
    // Event 10 min ago: past the stale threshold, under the TTL.
    write_atomic(
        &p,
        &file_aged(SessionStatus::Working, Duration::from_secs(600)),
    )
    .unwrap();

    let loaded = run(dir.path(), &policy()).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].status, SessionStatus::Connecting);
    // Reset writes the reset version back to disk.
    let on_disk = read(&p).unwrap().unwrap();
    assert_eq!(on_disk.status, SessionStatus::Connecting);
}

/// Regression: a stale file that recorded a blocking notification
/// subtype (e.g. `idle_prompt`) must not carry it through `Reset`.
/// The rewrite fires the status watcher in every running daruda
/// instance, and a preserved subtype re-raises a desktop push for
/// a prompt that is long gone.
#[test]
fn run_reset_clears_stale_notification() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = path_for(dir.path(), "stale-notif");
    let mut f = file_aged(SessionStatus::Idle, Duration::from_secs(600));
    f.notification = Some(daruda_agent::hooks::events::NotificationType::IdlePrompt);
    write_atomic(&p, &f).unwrap();

    let loaded = run(dir.path(), &policy()).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].notification, None);
    let on_disk = read(&p).unwrap().unwrap();
    assert_eq!(on_disk.notification, None);
}

/// What a later window reads: every file as it is, stale ones included,
/// and nothing on disk touched.
#[test]
fn load_reads_without_resetting_or_deleting() {
    let dir = tempfile::TempDir::new().unwrap();
    let stale = path_for(dir.path(), "stale");
    let old = path_for(dir.path(), "old");
    write_atomic(
        &stale,
        &file_aged(SessionStatus::Working, Duration::from_secs(600)),
    )
    .unwrap();
    write_atomic(
        &old,
        &file_aged(SessionStatus::Idle, Duration::from_secs(8 * 24 * 60 * 60)),
    )
    .unwrap();

    let loaded = load(dir.path()).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(
        read(&stale).unwrap().unwrap().status,
        SessionStatus::Working
    );
    assert!(old.exists());
}

#[test]
fn run_deletes_old_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = path_for(dir.path(), "old");
    // Event 8 days ago: past the 7-day TTL.
    write_atomic(
        &p,
        &file_aged(SessionStatus::Idle, Duration::from_secs(8 * 24 * 60 * 60)),
    )
    .unwrap();

    let loaded = run(dir.path(), &policy()).unwrap();
    assert!(loaded.is_empty());
    assert!(!p.exists());
}

/// Regression: a perpetually-idle session repeatedly reset to
/// `Connecting` keeps a fresh mtime (every `Reset` rewrites it) but an
/// old event timestamp. It must still age out by the event timestamp,
/// not live forever because the rewrite touched the mtime.
#[test]
fn run_ages_by_event_timestamp_not_mtime() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = path_for(dir.path(), "immortal");
    // 30-day-old event time, but `write_atomic` gives it a brand-new
    // mtime — the exact shape of an accumulated stale husk.
    write_atomic(
        &p,
        &file_aged(
            SessionStatus::Connecting,
            Duration::from_secs(30 * 24 * 60 * 60),
        ),
    )
    .unwrap();

    let loaded = run(dir.path(), &policy()).unwrap();
    assert!(
        loaded.is_empty(),
        "stale-by-timestamp file must be deleted regardless of fresh mtime"
    );
    assert!(!p.exists());
}

/// The inverse: a recently-active session with an ancient mtime (e.g.
/// restored from a backup) is kept, proving the event timestamp — not
/// the mtime — is the age basis.
#[test]
fn run_keeps_recent_event_even_with_old_mtime() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = path_for(dir.path(), "recent");
    write_atomic(
        &p,
        &file_aged(SessionStatus::Working, Duration::from_secs(60)),
    )
    .unwrap();

    let old = SystemTime::now() - Duration::from_secs(30 * 24 * 60 * 60);
    if !force_mtime(&p, old) {
        return; // platform skipped
    }

    let loaded = run(dir.path(), &policy()).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].status, SessionStatus::Working);
    assert!(p.exists());
}

#[test]
fn run_skips_malformed_recent_file_in_place() {
    // A malformed file with a fresh mtime is left alone — could be
    // mid-write. Don't load it, but don't delete it either.
    let dir = tempfile::TempDir::new().unwrap();
    let p = path_for(dir.path(), "wip");
    fs::write(&p, b"{ partial").unwrap();
    let loaded = run(dir.path(), &policy()).unwrap();
    assert!(loaded.is_empty());
    assert!(p.exists());
}

#[test]
fn run_preserves_old_lock_identity_even_while_it_is_held() {
    let dir = tempfile::TempDir::new().unwrap();
    let stale_lock = dir.path().join("orphan.lock");
    let fresh_lock = dir.path().join("active.lock");
    fs::write(&stale_lock, b"").unwrap();
    fs::write(&fresh_lock, b"").unwrap();

    let old = SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60);
    if !force_mtime(&stale_lock, old) {
        return; // platform skipped
    }

    let held = fs::File::options()
        .read(true)
        .write(true)
        .open(&stale_lock)
        .unwrap();
    held.lock().unwrap();
    run(dir.path(), &policy()).unwrap();
    assert!(
        stale_lock.exists(),
        "age cannot prove that a lock is orphaned"
    );
    let contender = fs::File::options()
        .read(true)
        .write(true)
        .open(&stale_lock)
        .unwrap();
    assert!(matches!(
        contender.try_lock(),
        Err(fs::TryLockError::WouldBlock)
    ));
    assert!(fresh_lock.exists(), "fresh lock should be left alone");
}

#[test]
fn run_does_not_reset_or_delete_a_status_while_its_writer_holds_the_lock() {
    use daruda_agent::hooks::status_file::lock_path_for;
    let dir = tempfile::TempDir::new().unwrap();
    for (id, age) in [("stale", 600), ("expired", 8 * 24 * 60 * 60)] {
        let path = path_for(dir.path(), id);
        let mut old = file_aged(SessionStatus::Working, Duration::from_secs(age));
        old.session_id = id.into();
        write_atomic(&path, &old).unwrap();
        let lock = fs::File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path_for(dir.path(), id))
            .unwrap();
        lock.lock().unwrap();
        assert!(run(dir.path(), &policy()).unwrap().is_empty());
        assert_eq!(read(&path).unwrap().unwrap().status, SessionStatus::Working);
        let fresh = StatusFile::new_hook(id, "/tmp/x", SessionStatus::ExecutingTool, "PreToolUse");
        write_atomic(&path, &fresh).unwrap();
        drop(lock);
        let loaded = run(dir.path(), &policy()).unwrap();
        assert!(
            loaded
                .iter()
                .any(|f| f.session_id == id && f.status == SessionStatus::ExecutingTool)
        );
        fs::remove_file(&path).unwrap();
    }
}

#[test]
fn run_skips_non_lock_non_json_files() {
    // The sweep must not touch unrelated extensions. Status JSON
    // sweep already filters by `.json`, lock sweep by `.lock` —
    // a stray README or `.tmp` should survive both.
    let dir = tempfile::TempDir::new().unwrap();
    let stray = dir.path().join("README.md");
    fs::write(&stray, b"hello").unwrap();
    run(dir.path(), &policy()).unwrap();
    assert!(stray.exists());
}
