//! File-system watcher for `panels.json` live reload.
//!
//! Mirrors `config_watcher` but targets the bottom-dock panels file.
//! On each debounced reload signal, `watchers_lifecycle::spawn_panels_reload`
//! reloads `PanelsState` from disk and pushes it to every open
//! `Workspace`. Self-write suppression is handled by comparing the
//! reloaded JSON against the current state and skipping the notify
//! if they match — robust against both daruda's own writes and
//! external no-op editor saves.

use std::sync::mpsc;
use std::time::Duration;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;

use crate::path_ext::PathExt;

const DEBOUNCE: Duration = Duration::from_millis(200);

/// Spawn a watcher thread that yields one debounced signal per
/// settling point on `panels.json`. The watcher runs until the
/// returned receiver is dropped.
pub fn spawn_panels_watcher() -> mpsc::Receiver<()> {
    use notify::{RecursiveMode, Watcher};

    let (tx, rx) = mpsc::channel();
    let panels_path =
        daruda_store::panels::panels_path_in(&daruda_store::persistence::default_data_dir());

    std::thread::spawn(move || {
        // Watch the parent directory — atomic-rename saves (which is
        // what daruda itself does via `tempfile + persist`) replace
        // the inode, so a direct file watch loses its target.
        let watch_dir = panels_path.parent_or_current().to_path_buf();
        let file_name = panels_path.file_name().map(|n| n.to_os_string());

        // Ensure the directory exists so notify can attach to it
        // (otherwise the very first watch attempt fails on a fresh
        // install before daruda has written anything).
        if let Err(e) = std::fs::create_dir_all(&watch_dir) {
            LogWriter::log(
                ErrorReport::new("panels watcher mkdir failed")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("path", redact_home(&watch_dir))
                    .dedup("panels.watcher.mkdir")
                    .build(),
            );
        }

        let tx_clone = tx.clone();
        let mut watcher =
            match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    let is_relevant = file_name.as_ref().is_none_or(|target| {
                        event
                            .paths
                            .iter()
                            .any(|p| p.file_name() == Some(target.as_os_str()))
                    });
                    if is_relevant {
                        let _ = tx_clone.send(());
                    }
                }
            }) {
                Ok(w) => w,
                Err(e) => {
                    LogWriter::log(
                        ErrorReport::new("Panels watcher disabled — FS watcher init failed")
                            .severity(ErrorSeverity::Warning)
                            .from_error(&e)
                            .at(file!(), line!())
                            .dedup("panels.watcher.init")
                            .build(),
                    );
                    return;
                }
            };

        if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
            LogWriter::log(
                ErrorReport::new("Panels watcher disabled — could not attach to directory")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("path", redact_home(&watch_dir))
                    .dedup("panels.watcher.attach")
                    .build(),
            );
            return;
        }

        loop {
            std::thread::park();
        }
    });

    let (debounced_tx, debounced_rx) = mpsc::channel();
    std::thread::spawn(move || {
        loop {
            if rx.recv().is_err() {
                break;
            }
            std::thread::sleep(DEBOUNCE);
            while rx.try_recv().is_ok() {}
            if debounced_tx.send(()).is_err() {
                break;
            }
        }
    });

    debounced_rx
}
