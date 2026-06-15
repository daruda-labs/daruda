//! Filesystem watcher for `~/.daruda/status/`.
//!
//! Mirrors the [`super::handler`]'s atomic writes back into the
//! GPUI process: every status-file create / modify / remove translates
//! to a [`StatusEvent`] on a `mpsc::Receiver`, which the main loop
//! pumps into each open Workspace's `ClaudeStatusStore`.
//!
//! Same shape as `crate::config_watcher` — non-recursive watch on the
//! parent directory, with rename-via-tempfile handled by listening to
//! both Create and Modify event kinds.
//!
//! FSEvents rescan/drop recovery (sleep/wake) is handled by
//! [`crate::dir_watch::spawn_dir_watcher`] via [`enumerate_status_dir`].

use std::path::PathBuf;
use std::sync::mpsc;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;
use notify::RecursiveMode;

/// One status-file change.
#[derive(Clone, Debug)]
pub enum StatusEvent {
    /// File was created or modified — read it back and `update` the store.
    Changed(PathBuf),
    /// File was removed (typically `SessionEnd` via the hook handler).
    /// `session_id` is the filename stem so the store can drop the
    /// entry without re-reading the (now absent) file.
    Removed { session_id: String },
}

/// Classify a single notify event into zero or more [`StatusEvent`]s.
///
/// Handles `Remove` → `Removed` and `Create`/`Modify` → `Changed` for `.json`
/// files. All other event kinds (including unclassified FSEvents noise) are
/// silently skipped; rescan recovery is handled upstream by
/// [`crate::dir_watch::spawn_dir_watcher`].
fn classify_status_event(event: &notify::Event) -> Vec<StatusEvent> {
    use notify::EventKind;

    let is_remove = matches!(event.kind, EventKind::Remove(_));
    let is_change = matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_));
    if !is_remove && !is_change {
        return vec![];
    }

    let mut out = Vec::new();
    for path in event.paths.iter() {
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let item = if is_remove {
            let session_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            StatusEvent::Removed { session_id }
        } else {
            StatusEvent::Changed(path.clone())
        };
        out.push(item);
    }
    out
}

/// Enumerate all `.json` files in `dir` and emit a [`StatusEvent::Changed`]
/// for each. Called on FSEvents rescan (sleep/wake) to recover lost events.
/// Returns an empty vec on any `read_dir` error.
fn enumerate_status_dir(dir: &std::path::Path) -> Vec<StatusEvent> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()? == "json" {
                Some(StatusEvent::Changed(path))
            } else {
                None
            }
        })
        .collect()
}

/// Start watching `dir`. Creates the directory if missing so the
/// notify backend has something to subscribe to. Returns the event
/// receiver plus a [`crate::dir_watch::DirWatcher`] handle that the caller
/// must keep alive for as long as it wants events (this watcher is app-global,
/// so the caller parks the handle for the process lifetime).
///
/// On FSEvents rescan events (sleep/wake), the directory is re-enumerated via
/// [`enumerate_status_dir`] so events lost during the gap are recovered.
pub fn spawn_status_watcher(
    dir: PathBuf,
) -> (mpsc::Receiver<StatusEvent>, crate::dir_watch::DirWatcher) {
    // Ensure the directory exists; otherwise notify has nothing to
    // watch and the user's first hook write would race against the
    // watcher startup.
    if let Err(e) = std::fs::create_dir_all(&dir) {
        LogWriter::log(
            ErrorReport::new("hooks watcher mkdir failed")
                .severity(ErrorSeverity::Warning)
                .from_error(&e)
                .at(file!(), line!())
                .with_context("path", redact_home(&dir))
                .dedup("hooks.watcher.mkdir")
                .build(),
        );
    }

    let anchors = [dir.clone()];
    crate::dir_watch::spawn_dir_watcher(
        &anchors,
        RecursiveMode::NonRecursive,
        classify_status_event,
        move || enumerate_status_dir(&dir),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_claude::SessionStatus;
    use daruda_claude::hooks::status_file::{StatusFile, path_for, write_atomic};
    use std::time::{Duration, Instant};

    #[test]
    fn enumerate_status_dir_returns_changed_for_json_only() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.json"), "{}").unwrap();
        std::fs::write(dir.path().join("b.json"), "{}").unwrap();
        std::fs::write(dir.path().join("note.txt"), "noise").unwrap();

        let events = enumerate_status_dir(dir.path());

        assert_eq!(
            events.len(),
            2,
            "expected 2 Changed events (json only), got {events:?}"
        );
        for ev in &events {
            match ev {
                StatusEvent::Changed(path) => {
                    assert_eq!(
                        path.extension().and_then(|e| e.to_str()),
                        Some("json"),
                        "non-json file leaked: {path:?}"
                    );
                }
                StatusEvent::Removed { .. } => {
                    panic!("enumerate should never emit Removed, got {ev:?}");
                }
            }
        }
        // note.txt must not appear
        let has_txt = events.iter().any(|ev| match ev {
            StatusEvent::Changed(p) => p.file_name().and_then(|n| n.to_str()) == Some("note.txt"),
            _ => false,
        });
        assert!(!has_txt, "note.txt should be excluded");
    }

    #[test]
    fn classify_create_produces_changed() {
        use notify::EventKind;
        use notify::event::CreateKind;
        let path = std::path::PathBuf::from("/tmp/ses-1.json");
        let mut ev = notify::Event::new(EventKind::Create(CreateKind::Any));
        ev.paths.push(path.clone());
        assert!(matches!(&classify_status_event(&ev)[..], [StatusEvent::Changed(p)] if *p == path));
    }

    #[test]
    fn classify_remove_produces_removed_with_stem() {
        use notify::EventKind;
        use notify::event::RemoveKind;
        let mut ev = notify::Event::new(EventKind::Remove(RemoveKind::Any));
        ev.paths.push(std::path::PathBuf::from("/tmp/ses-42.json"));
        assert!(
            matches!(&classify_status_event(&ev)[..], [StatusEvent::Removed { session_id }] if session_id == "ses-42")
        );
    }

    #[test]
    fn classify_non_json_is_skipped() {
        use notify::EventKind;
        use notify::event::CreateKind;
        let mut ev = notify::Event::new(EventKind::Create(CreateKind::Any));
        ev.paths.push(std::path::PathBuf::from("/tmp/readme.txt"));
        assert!(classify_status_event(&ev).is_empty());
    }

    /// Wait up to `timeout` for an event matching `pred`. Drains other
    /// noise events (FSEvents on macOS sometimes emits Create then
    /// Modify for one logical write).
    fn wait_for<F>(
        rx: &mpsc::Receiver<StatusEvent>,
        timeout: Duration,
        mut pred: F,
    ) -> Option<StatusEvent>
    where
        F: FnMut(&StatusEvent) -> bool,
    {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(ev) = rx.recv_timeout(Duration::from_millis(50))
                && pred(&ev)
            {
                return Some(ev);
            }
        }
        None
    }

    #[test]
    #[ignore = "FSEvents-dependent — run via `cargo test --ignored`"]
    fn watcher_emits_changed_on_atomic_write() {
        let _g = crate::hooks::tests_common::fsevent_serial();
        let dir = tempfile::TempDir::new().unwrap();
        let (rx, _watcher) = spawn_status_watcher(dir.path().to_path_buf());

        // Give the watcher a moment to attach.
        std::thread::sleep(Duration::from_millis(150));

        let file = StatusFile::new_hook("ses-1", "/tmp/x", SessionStatus::Working, "PreToolUse");
        let p = path_for(dir.path(), "ses-1");
        write_atomic(&p, &file).unwrap();

        let ev = wait_for(
            &rx,
            Duration::from_secs(3),
            |e| matches!(e, StatusEvent::Changed(path) if path.file_stem().and_then(|s| s.to_str()) == Some("ses-1")),
        );
        assert!(ev.is_some(), "did not receive Changed event for ses-1");
    }

    #[test]
    #[ignore = "FSEvents-dependent — run via `cargo test --ignored`"]
    fn watcher_emits_removed_on_delete() {
        let _g = crate::hooks::tests_common::fsevent_serial();
        let dir = tempfile::TempDir::new().unwrap();
        let (rx, _watcher) = spawn_status_watcher(dir.path().to_path_buf());

        std::thread::sleep(Duration::from_millis(150));

        let file = StatusFile::new_hook("ses-2", "/tmp/y", SessionStatus::Idle, "Stop");
        let p = path_for(dir.path(), "ses-2");
        write_atomic(&p, &file).unwrap();

        // Drain the create/modify burst.
        std::thread::sleep(Duration::from_millis(150));
        while rx.try_recv().is_ok() {}

        std::fs::remove_file(&p).unwrap();

        let ev = wait_for(
            &rx,
            Duration::from_secs(3),
            |e| matches!(e, StatusEvent::Removed { session_id, .. } if session_id == "ses-2"),
        );
        assert!(ev.is_some(), "did not receive Removed event for ses-2");
    }

    #[test]
    #[ignore = "FSEvents-dependent — run via `cargo test --ignored`"]
    fn non_json_files_are_ignored() {
        let _g = crate::hooks::tests_common::fsevent_serial();
        let dir = tempfile::TempDir::new().unwrap();
        let (rx, _watcher) = spawn_status_watcher(dir.path().to_path_buf());
        std::thread::sleep(Duration::from_millis(150));

        std::fs::write(dir.path().join("readme.txt"), "noise").unwrap();
        std::fs::write(dir.path().join("config.toml"), "noise").unwrap();

        // Brief settling period; we should NOT receive any events.
        std::thread::sleep(Duration::from_millis(300));
        if let Ok(ev) = rx.try_recv() {
            panic!("expected no events for non-json files; got {ev:?}");
        }
    }
}
