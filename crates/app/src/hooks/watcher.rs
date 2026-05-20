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

use std::path::PathBuf;
use std::sync::mpsc;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;

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

/// Start watching `dir`. Creates the directory if missing so the
/// notify backend has something to subscribe to. Returns a receiver;
/// the watcher thread keeps running until the receiver is dropped.
pub fn spawn_status_watcher(dir: PathBuf) -> mpsc::Receiver<StatusEvent> {
    use notify::{EventKind, RecursiveMode, Watcher};

    let (tx, rx) = mpsc::channel();

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

    std::thread::spawn(move || {
        let tx_inner = tx.clone();
        let mut watcher =
            match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                let Ok(event) = res else {
                    return;
                };

                let is_remove = matches!(event.kind, EventKind::Remove(_));
                let is_change = matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_));
                if !is_remove && !is_change {
                    return;
                }

                for path in event.paths.iter() {
                    if path.extension().is_none_or(|e| e != "json") {
                        continue;
                    }
                    let to_send = if is_remove {
                        let session_id = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or_default()
                            .to_string();
                        StatusEvent::Removed { session_id }
                    } else {
                        StatusEvent::Changed(path.clone())
                    };
                    let _ = tx_inner.send(to_send);
                }
            }) {
                Ok(w) => w,
                Err(_) => return,
            };

        if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
            return;
        }

        // Park forever; receiver-drop cleans up via SendError on the
        // closure above.
        loop {
            std::thread::park();
        }
    });

    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_claude::SessionStatus;
    use daruda_claude::hooks::status_file::{StatusFile, path_for, write_atomic};
    use std::time::{Duration, Instant};

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
        let rx = spawn_status_watcher(dir.path().to_path_buf());

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
        let rx = spawn_status_watcher(dir.path().to_path_buf());

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
        let rx = spawn_status_watcher(dir.path().to_path_buf());
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
