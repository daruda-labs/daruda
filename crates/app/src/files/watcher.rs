//! File-system watcher for the left-dock Files view.
//!
//! Per-lane `notify::RecommendedWatcher` with recursive watch (macOS
//! FSEvents). Raw events are debounced on a dedicated thread (30 ms
//! window); the thread blocks on `recv()` so it parks at zero CPU when
//! the filesystem is idle. Bursts above `BULK_THRESHOLD` collapse to a
//! single `Bulk` event so a `git checkout` does not produce one reload
//! per file.
//!
//! Paths inside `.git/` are filtered out at classification time —
//! `git status` itself writes to `.git/index` (stat-cache update),
//! and forwarding those events would let `refresh_git_status` trigger
//! itself in a loop, pinning idle CPU near 3.5 %. Working-tree changes
//! (including `git checkout` / `git pull` results) live outside
//! `.git/`, so the filter does not blind us to user-visible state.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(30);
const BULK_THRESHOLD: usize = 100;

/// Debounced filesystem event delivered to `Workspace`.
#[derive(Debug, Clone)]
pub enum DebouncedEvent {
    /// Path set inside a single 30 ms window, excluding deletions.
    /// Caller reloads each parent directory.
    Changed { paths: Vec<PathBuf> },
    /// Paths that `notify` reported with `EventKind::Remove(_)`.
    /// Caller removes the corresponding subtree directly — avoids the
    /// NotFound race window where a follow-up parent reload would race
    /// the deletion.
    Removed { paths: Vec<PathBuf> },
    /// More than `BULK_THRESHOLD` paths reported in one window —
    /// caller should reload from the root + every currently-expanded
    /// directory.
    Bulk,
    /// Fatal watcher error (e.g. backing kernel queue lost). Surfaces
    /// in the status bar; manual refresh is the fallback.
    Error(String),
}

/// Owns the `notify::Watcher` plus the debounce thread. The watcher
/// stops when this struct is dropped.
pub struct FileTreeWatcher {
    /// `notify::Watcher`; stops the kernel-side watch on drop. Held
    /// in a field so it lives as long as `FileTreeWatcher`.
    _watcher: RecommendedWatcher,
    /// Receiver of debounced events. The `Workspace` polling task
    /// drains this on each tick.
    pub events_rx: mpsc::Receiver<DebouncedEvent>,
}

impl FileTreeWatcher {
    pub fn new(root: PathBuf) -> Result<Self, notify::Error> {
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let mut watcher: RecommendedWatcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                match res {
                    Ok(ev) => {
                        let _ = raw_tx.send(Ok(ev));
                    }
                    Err(e) => {
                        // Kernel queue overflow or permission error: forward
                        // the message so the debounce loop can emit an Error
                        // event to the Workspace, which surfaces it in the
                        // status bar.
                        let _ = raw_tx.send(Err(e.to_string()));
                    }
                }
            })?;
        watcher.watch(&root, RecursiveMode::Recursive)?;

        let (out_tx, out_rx) = mpsc::channel::<DebouncedEvent>();
        std::thread::spawn(move || debounce_loop(raw_rx, out_tx));
        Ok(Self {
            _watcher: watcher,
            events_rx: out_rx,
        })
    }
}

fn debounce_loop(
    raw_rx: mpsc::Receiver<Result<Event, String>>,
    out_tx: mpsc::Sender<DebouncedEvent>,
) {
    loop {
        // Block until the first event arrives — thread parks here
        // when the filesystem is idle (0 CPU).
        let first = match raw_rx.recv() {
            Ok(Ok(ev)) => ev,
            Ok(Err(msg)) => {
                let _ = out_tx.send(DebouncedEvent::Error(msg));
                continue;
            }
            Err(_) => return,
        };
        let mut changed: Vec<PathBuf> = Vec::new();
        let mut removed: Vec<PathBuf> = Vec::new();
        classify(first, &mut changed, &mut removed);
        let deadline = Instant::now() + DEBOUNCE_WINDOW;

        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match raw_rx.recv_timeout(deadline - now) {
                Ok(Ok(ev)) => classify(ev, &mut changed, &mut removed),
                Ok(Err(msg)) => {
                    flush(&mut changed, &mut removed, &out_tx);
                    let _ = out_tx.send(DebouncedEvent::Error(msg));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    flush(&mut changed, &mut removed, &out_tx);
                    return;
                }
            }
        }
        flush(&mut changed, &mut removed, &out_tx);
    }
}

/// Route `notify::EventKind::Remove(_)` to `removed`; everything else
/// (Create / Modify / Access / Any / Other) to `changed`. Path-level
/// granularity — a single `Event` may carry multiple paths, all of the
/// same kind by notify's contract. Paths inside `.git/` are dropped at
/// this step (see module docs).
fn classify(ev: Event, changed: &mut Vec<PathBuf>, removed: &mut Vec<PathBuf>) {
    let bucket = if matches!(ev.kind, EventKind::Remove(_)) {
        removed
    } else {
        changed
    };
    bucket.extend(ev.paths.into_iter().filter(|p| !path_inside_git_dir(p)));
}

/// True when any component of `path` equals `.git` — matches `.git/`
/// at any depth. `.gitignore` and similar top-level dotfiles are
/// unaffected because they are not directories.
fn path_inside_git_dir(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str() == std::ffi::OsStr::new(".git"))
}

fn flush(
    changed: &mut Vec<PathBuf>,
    removed: &mut Vec<PathBuf>,
    tx: &mpsc::Sender<DebouncedEvent>,
) {
    let total = changed.len() + removed.len();
    if total == 0 {
        return;
    }
    if total >= BULK_THRESHOLD {
        changed.clear();
        removed.clear();
        let _ = tx.send(DebouncedEvent::Bulk);
        return;
    }
    if !removed.is_empty() {
        let _ = tx.send(DebouncedEvent::Removed {
            paths: std::mem::take(removed),
        });
    }
    if !changed.is_empty() {
        let _ = tx.send(DebouncedEvent::Changed {
            paths: std::mem::take(changed),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::EventKind;
    use notify::event::EventAttributes;

    fn dummy_event(path: &str) -> Result<Event, String> {
        Ok(Event {
            kind: EventKind::Any,
            paths: vec![PathBuf::from(path)],
            attrs: EventAttributes::new(),
        })
    }

    fn remove_event(path: &str) -> Result<Event, String> {
        Ok(Event {
            kind: EventKind::Remove(notify::event::RemoveKind::Any),
            paths: vec![PathBuf::from(path)],
            attrs: EventAttributes::new(),
        })
    }

    #[test]
    fn debounce_loop_emits_changed_for_small_burst() {
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let (out_tx, out_rx) = mpsc::channel::<DebouncedEvent>();
        raw_tx.send(dummy_event("a")).unwrap();
        raw_tx.send(dummy_event("b")).unwrap();
        drop(raw_tx);
        debounce_loop(raw_rx, out_tx);

        match out_rx.try_recv().unwrap() {
            DebouncedEvent::Changed { paths } => {
                assert_eq!(paths.len(), 2);
                assert_eq!(paths[0], PathBuf::from("a"));
                assert_eq!(paths[1], PathBuf::from("b"));
            }
            other => panic!("expected Changed, got {other:?}"),
        }
    }

    #[test]
    fn debounce_loop_collapses_to_bulk_above_threshold() {
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let (out_tx, out_rx) = mpsc::channel::<DebouncedEvent>();
        for i in 0..(BULK_THRESHOLD + 5) {
            raw_tx.send(dummy_event(&format!("p{i}"))).unwrap();
        }
        drop(raw_tx);
        debounce_loop(raw_rx, out_tx);

        assert!(matches!(out_rx.try_recv().unwrap(), DebouncedEvent::Bulk));
    }

    #[test]
    fn debounce_loop_returns_on_disconnect() {
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let (out_tx, _out_rx) = mpsc::channel::<DebouncedEvent>();
        // `_raw_tx` would *bind* the sender for the rest of the scope —
        // underscore-prefix names live until the function returns, only
        // a bare `_` is dropped immediately. Drop explicitly so
        // `debounce_loop` sees a disconnected channel and returns.
        drop(raw_tx);
        debounce_loop(raw_rx, out_tx); // must return promptly
    }

    #[test]
    fn debounce_loop_handles_event_with_empty_paths() {
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let (out_tx, out_rx) = mpsc::channel::<DebouncedEvent>();
        raw_tx
            .send(Ok(Event {
                kind: EventKind::Any,
                paths: vec![],
                attrs: EventAttributes::new(),
            }))
            .unwrap();
        drop(raw_tx);
        debounce_loop(raw_rx, out_tx);
        // No paths in the only event → flush no-ops → out channel empty.
        assert!(out_rx.try_recv().is_err());
    }

    #[test]
    fn debounce_loop_separates_remove_from_change() {
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let (out_tx, out_rx) = mpsc::channel::<DebouncedEvent>();
        raw_tx.send(remove_event("gone")).unwrap();
        raw_tx.send(dummy_event("kept")).unwrap();
        drop(raw_tx);
        debounce_loop(raw_rx, out_tx);

        // Removed emitted first, then Changed.
        match out_rx.try_recv().unwrap() {
            DebouncedEvent::Removed { paths } => {
                assert_eq!(paths, vec![PathBuf::from("gone")]);
            }
            other => panic!("expected Removed first, got {other:?}"),
        }
        match out_rx.try_recv().unwrap() {
            DebouncedEvent::Changed { paths } => {
                assert_eq!(paths, vec![PathBuf::from("kept")]);
            }
            other => panic!("expected Changed second, got {other:?}"),
        }
    }

    #[test]
    fn debounce_loop_collapses_mixed_burst_to_bulk() {
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let (out_tx, out_rx) = mpsc::channel::<DebouncedEvent>();
        // Half removes, half changes — total ≥ threshold collapses
        // both buckets to a single Bulk.
        for i in 0..(BULK_THRESHOLD / 2 + 1) {
            raw_tx.send(remove_event(&format!("r{i}"))).unwrap();
            raw_tx.send(dummy_event(&format!("c{i}"))).unwrap();
        }
        drop(raw_tx);
        debounce_loop(raw_rx, out_tx);

        assert!(matches!(out_rx.try_recv().unwrap(), DebouncedEvent::Bulk));
        assert!(out_rx.try_recv().is_err(), "Bulk must be the only event");
    }

    #[test]
    fn classify_drops_git_internal_paths() {
        let ev = Event {
            kind: EventKind::Any,
            paths: vec![
                PathBuf::from("/repo/.git/index"),
                PathBuf::from("/repo/.git/refs/heads/main"),
                PathBuf::from("/repo/src/main.rs"),
            ],
            attrs: EventAttributes::new(),
        };
        let mut changed: Vec<PathBuf> = Vec::new();
        let mut removed: Vec<PathBuf> = Vec::new();
        classify(ev, &mut changed, &mut removed);
        assert_eq!(changed, vec![PathBuf::from("/repo/src/main.rs")]);
        assert!(removed.is_empty());
    }

    #[test]
    fn debounce_loop_swallows_pure_git_burst() {
        // Bursts containing only `.git/` paths must produce no output
        // events — the previous behaviour fed `refresh_git_status` a
        // continuous trigger that pinned ~3.5 % idle CPU.
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let (out_tx, out_rx) = mpsc::channel::<DebouncedEvent>();
        raw_tx.send(dummy_event("/repo/.git/index")).unwrap();
        raw_tx
            .send(dummy_event("/repo/.git/refs/heads/main"))
            .unwrap();
        raw_tx.send(remove_event("/repo/.git/HEAD.lock")).unwrap();
        drop(raw_tx);
        debounce_loop(raw_rx, out_tx);
        assert!(out_rx.try_recv().is_err(), "no event must be emitted");
    }

    #[test]
    fn debounce_loop_preserves_mixed_burst_outside_paths() {
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let (out_tx, out_rx) = mpsc::channel::<DebouncedEvent>();
        raw_tx.send(dummy_event("/repo/.git/index")).unwrap();
        raw_tx.send(dummy_event("/repo/Cargo.toml")).unwrap();
        drop(raw_tx);
        debounce_loop(raw_rx, out_tx);
        match out_rx.try_recv().unwrap() {
            DebouncedEvent::Changed { paths } => {
                assert_eq!(paths, vec![PathBuf::from("/repo/Cargo.toml")]);
            }
            other => panic!("expected Changed with one path, got {other:?}"),
        }
    }

    #[test]
    fn path_inside_git_dir_handles_nested_and_dotfile_edge_cases() {
        assert!(path_inside_git_dir(Path::new("/repo/.git/HEAD")));
        assert!(path_inside_git_dir(Path::new("/repo/sub/.git/HEAD")));
        // `.gitignore` is a file at repo root, not a `.git` directory.
        assert!(!path_inside_git_dir(Path::new("/repo/.gitignore")));
        // `.gitmodules` likewise must not be filtered.
        assert!(!path_inside_git_dir(Path::new("/repo/.gitmodules")));
        // Regular source path: untouched.
        assert!(!path_inside_git_dir(Path::new("/repo/src/main.rs")));
    }

    #[test]
    fn debounce_loop_forwards_watcher_error() {
        let (raw_tx, raw_rx) = mpsc::channel::<Result<Event, String>>();
        let (out_tx, out_rx) = mpsc::channel::<DebouncedEvent>();
        raw_tx
            .send(Err("kernel queue overflow".to_string()))
            .unwrap();
        drop(raw_tx);
        debounce_loop(raw_rx, out_tx);

        match out_rx.try_recv().unwrap() {
            DebouncedEvent::Error(msg) => assert_eq!(msg, "kernel queue overflow"),
            other => panic!("expected Error, got {other:?}"),
        }
    }
}
