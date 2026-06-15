//! Filesystem watcher for the TaskEdit pane's prompt markdown file
//! (R-20 / I-13):
//!
//! 1. **`dir_watch`** owns the `notify::Watcher` (via the returned
//!    [`crate::dir_watch::DirWatcher`] handle) and forwards events for our
//!    target path into a raw channel; FSEvents drops (sleep/wake) emit one
//!    signal so the prompt is re-read.
//! 2. **Debounce thread** collapses bursts (vim's atomic-rename
//!    emits ≥3 events per save) into one signal per [`DEBOUNCE`]
//!    window.
//!
//! Each TaskEdit pane owns its own `PromptFileWatcherHandle` — dropping the
//! handle drops the `DirWatcher` (stopping the watch), which disconnects the
//! raw channel and ends the debounce thread. The GPUI-side pump task that
//! consumes the debounced channel lives on the same `Pane` so its lifetime is
//! tied to the pane.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

/// Debounce window — atomic-rename bursts finish well inside this.
const DEBOUNCE: Duration = Duration::from_millis(50);

/// Caller-side handle. Dropping it drops the `DirWatcher` (stops the watch);
/// the debounce thread then exits once the raw channel disconnects.
pub(in crate::workspace) struct PromptFileWatcherHandle {
    _watcher: crate::dir_watch::DirWatcher,
}

/// Spawn a watcher on `path`. Returns the debounced event receiver +
/// the handle that owns the watcher lifetime.
///
/// Watches the *parent* directory so we still see create events for
/// files that didn't exist at spawn time (vim's `.swp` → rename
/// pattern). Events whose path doesn't canonicalise to `path` are
/// filtered out by `classify`.
pub(in crate::workspace) fn spawn(path: PathBuf) -> (mpsc::Receiver<()>, PromptFileWatcherHandle) {
    use notify::RecursiveMode;

    let (event_tx, event_rx) = mpsc::channel::<()>();

    // Canonicalise once (resolves /var/folders → /private/var/folders
    // on macOS). When the target doesn't exist yet, fall back to the
    // raw path; the watcher can still detect create events on the
    // parent dir and the comparison will line up on the first save.
    let canonical_target = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let target_for_thread = canonical_target.clone();
    let parent = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let classify = move |event: &notify::Event| {
        if !matches!(
            event.kind,
            notify::EventKind::Modify(_) | notify::EventKind::Create(_)
        ) {
            return vec![];
        }
        for p in &event.paths {
            let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            if canon == target_for_thread {
                return vec![()];
            }
        }
        vec![]
    };
    // Parent-dir subscription survives target-file create / rename;
    // NonRecursive is enough since our target sits directly inside.
    let anchors = [parent];
    let (raw_rx, watcher) = crate::dir_watch::spawn_dir_watcher(
        &anchors,
        RecursiveMode::NonRecursive,
        classify,
        || vec![()],
    );

    // Debounce thread.
    std::thread::spawn(move || {
        while let Ok(()) = raw_rx.recv() {
            // Drain bursts within the debounce window.
            let deadline = std::time::Instant::now() + DEBOUNCE;
            loop {
                let now = std::time::Instant::now();
                if now >= deadline {
                    break;
                }
                match raw_rx.recv_timeout(deadline - now) {
                    Ok(()) => continue,
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => {
                        let _ = event_tx.send(());
                        return;
                    }
                }
            }
            if event_tx.send(()).is_err() {
                return;
            }
        }
    });

    (event_rx, PromptFileWatcherHandle { _watcher: watcher })
}
