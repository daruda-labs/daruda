//! Filesystem watcher for the TaskEdit pane's prompt markdown file
//! (R-20 / I-13). Mirrors `hooks::mcp_watcher`'s 2-thread layout:
//!
//! 1. **FSEvent thread** owns the `notify::Watcher` and forwards
//!    raw events for our target path into a `mpsc` channel.
//! 2. **Debounce thread** collapses bursts (vim's atomic-rename
//!    emits ≥3 events per save) into one signal per [`DEBOUNCE`]
//!    window.
//!
//! Each TaskEdit pane owns its own `PromptFileWatcherHandle` —
//! dropping the handle stops both threads. The GPUI-side pump task
//! that consumes the debounced channel lives on the same `Pane` so
//! its lifetime is tied to the pane.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

/// Debounce window — atomic-rename bursts finish well inside this.
const DEBOUNCE: Duration = Duration::from_millis(50);

/// Caller-side handle. Dropping it closes the FS watcher thread; the
/// debounce thread exits once the raw channel disconnects.
pub(in crate::workspace) struct PromptFileWatcherHandle {
    _shutdown_tx: mpsc::Sender<()>,
}

/// Spawn a watcher on `path`. Returns the debounced event receiver +
/// the handle that owns the watcher lifetime.
///
/// Watches the *parent* directory so we still see create events for
/// files that didn't exist at spawn time (vim's `.swp` → rename
/// pattern). Events whose path doesn't canonicalise to `path` are
/// filtered out by the FS thread.
pub(in crate::workspace) fn spawn(path: PathBuf) -> (mpsc::Receiver<()>, PromptFileWatcherHandle) {
    use notify::{RecursiveMode, Watcher};

    let (raw_tx, raw_rx) = mpsc::channel::<()>();
    let (event_tx, event_rx) = mpsc::channel::<()>();
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

    // Canonicalise once (resolves /var/folders → /private/var/folders
    // on macOS). When the target doesn't exist yet, fall back to the
    // raw path; the FS thread can still detect create events on the
    // parent dir and the comparison will line up on the first save.
    let canonical_target = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let target_for_thread = canonical_target.clone();
    let parent = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    std::thread::spawn(move || {
        let raw_tx_inner = raw_tx.clone();

        let mut watcher =
            match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                let Ok(event) = res else {
                    return;
                };
                if !matches!(
                    event.kind,
                    notify::EventKind::Modify(_) | notify::EventKind::Create(_)
                ) {
                    return;
                }
                for p in &event.paths {
                    let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
                    if canon == target_for_thread {
                        let _ = raw_tx_inner.send(());
                        return;
                    }
                }
            }) {
                Ok(w) => w,
                Err(_) => return,
            };

        // Parent-dir subscription survives target-file create / rename;
        // NonRecursive is enough since our target sits directly inside.
        let _ = watcher.watch(&parent, RecursiveMode::NonRecursive);

        let _ = shutdown_rx.recv();
        drop(raw_tx);
    });

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

    (
        event_rx,
        PromptFileWatcherHandle {
            _shutdown_tx: shutdown_tx,
        },
    )
}
