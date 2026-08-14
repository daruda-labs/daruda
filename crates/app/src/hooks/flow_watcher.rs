//! Flow-definition watcher — `notify` subscriptions on the directories a
//! lane's flows can live in (repo, project, global).
//!
//! Unlike Skills and MCP, this one anchors on the flow directories
//! **themselves** and never ascends to a parent. A missing directory is left
//! unwatched, so a flow created from outside the app into a directory that did
//! not exist yet appears on the next lane switch or explicit reload rather than
//! at once. That bound is deliberate: ascending one level would mean recursively
//! watching `<data_dir>/`, where the log writer appends on every event, and
//! `<lane>/.daruda/`, where a running flow writes `flow-runs/**/run.yaml` — two
//! event storms in exchange for one uncommon case.
//!
//! Only *direct children* carrying a flow extension are reported, for the same
//! reason: the picker reads one directory level, so anything deeper is not a
//! flow this app can run.
//!
//! macOS quirk shared with the other watchers: FSEvents reports canonicalised
//! paths (`/var → /private/var`), so each anchor is canonicalised before the
//! per-event comparison.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

/// Coalescing window. Same value as the other watchers — an atomic-rename save
/// emits its burst well inside it, and the graph still reads as live.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// One coalesced signal that *something* about this lane's flows changed.
///
/// No path travels with it. Two reasons: a graph pane's reload is a byte
/// comparison against the text it already holds, so telling every pane costs a
/// read of a small file and repaints only what actually differs; and an atomic
/// save arrives as events for a temporary name, which a path filter would have
/// to guess at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowsEvent {
    Changed,
}

/// Caller-side handle. Dropping it drops the [`crate::dir_watch::DirWatcher`]
/// (stops the watch), which disconnects the raw channel and ends the debounce
/// thread. Re-spawned when the active lane changes.
pub struct FlowWatcherHandle {
    _watcher: crate::dir_watch::DirWatcher,
}

/// Spawn the watcher over `dirs` — the flow directories of the active lane, in
/// any order. `extensions` is what counts as a flow file, passed in rather than
/// known here so `workspace::flow_paths` stays the one place that decides.
///
/// Returns `(events, handle)`: the pump task takes ownership of `events`, while
/// the handle stays on `Workspace` so dropping it stops the worker thread.
pub fn spawn(
    dirs: Vec<PathBuf>,
    extensions: Vec<&'static str>,
) -> (mpsc::Receiver<FlowsEvent>, FlowWatcherHandle) {
    let (event_tx, event_rx) = mpsc::channel::<FlowsEvent>();

    let anchors: Vec<PathBuf> = dirs
        .iter()
        .filter(|d| d.is_dir())
        .map(|d| canonicalize_or_self(d))
        .collect();

    let matches = anchors.clone();
    let classify = move |event: &notify::Event| {
        let mut out = Vec::new();
        for path in &event.paths {
            if is_flow_in(path, &matches, &extensions) {
                out.push(FlowsEvent::Changed);
            }
        }
        out
    };
    // An FSEvents drop cannot say what changed, and the event carries no path
    // anyway — one signal is the whole recovery.
    let rescan = || vec![FlowsEvent::Changed];

    let (raw_rx, watcher) = crate::dir_watch::spawn_dir_watcher(
        &anchors,
        notify::RecursiveMode::NonRecursive,
        classify,
        rescan,
    );

    // Debounce thread: collapse each burst into one event.
    std::thread::spawn(move || {
        while raw_rx.recv().is_ok() {
            let deadline = std::time::Instant::now() + DEBOUNCE;
            loop {
                let now = std::time::Instant::now();
                if now >= deadline {
                    break;
                }
                match raw_rx.recv_timeout(deadline - now) {
                    Ok(_) => {}
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => {
                        // SILENT-OK: the watcher is gone; this last send is a
                        // courtesy and the receiver may already be dropped too.
                        let _ = event_tx.send(FlowsEvent::Changed);
                        return;
                    }
                }
            }
            if event_tx.send(FlowsEvent::Changed).is_err() {
                return;
            }
        }
    });

    (event_rx, FlowWatcherHandle { _watcher: watcher })
}

/// Is `path` a flow file sitting directly in one of `dirs`?
fn is_flow_in(path: &Path, dirs: &[PathBuf], extensions: &[&str]) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    if !dirs.iter().any(|d| d == parent) {
        return false;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| extensions.iter().any(|want| ext.eq_ignore_ascii_case(want)))
}

fn canonicalize_or_self(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flow_beside_the_watched_directory_is_not_one_of_ours() {
        let dirs = vec![PathBuf::from("/w/.daruda/flows")];
        assert!(is_flow_in(
            Path::new("/w/.daruda/flows/a.yaml"),
            &dirs,
            &["yaml"]
        ));
        assert!(!is_flow_in(
            Path::new("/w/.daruda/a.yaml"),
            &dirs,
            &["yaml"]
        ));
    }

    /// The case that decides the filter: the engine writes a `run.yaml` per
    /// step, and it must not read as a definition changing.
    #[test]
    fn a_runs_directory_write_is_not_a_definition_change() {
        let dirs = vec![PathBuf::from("/w/.daruda/flows")];
        assert!(!is_flow_in(
            Path::new("/w/.daruda/flow-runs/2026-08-13-1/run.yaml"),
            &dirs,
            &["yaml"]
        ));
    }

    #[test]
    fn only_the_extensions_the_picker_lists_count() {
        let dirs = vec![PathBuf::from("/w/flows")];
        assert!(is_flow_in(
            Path::new("/w/flows/a.yml"),
            &dirs,
            &["yaml", "yml"]
        ));
        assert!(!is_flow_in(
            Path::new("/w/flows/notes.md"),
            &dirs,
            &["yaml", "yml"]
        ));
        // A directory created inside the flows dir has no extension at all.
        assert!(!is_flow_in(
            Path::new("/w/flows/nested"),
            &dirs,
            &["yaml", "yml"]
        ));
    }

    /// FSEvents-dependent end to end: write a flow into a watched directory and
    /// wait for the coalesced signal.
    #[test]
    #[ignore = "FSEvents-dependent — run via `cargo test -- --ignored`"]
    fn a_write_into_a_watched_directory_reports_a_change() {
        let _g = crate::hooks::tests_common::fsevent_serial();
        let dir = tempfile::tempdir().unwrap();
        let flows = dir.path().join("flows");
        std::fs::create_dir_all(&flows).unwrap();

        let (rx, _handle) = spawn(vec![flows.clone()], vec!["yaml", "yml"]);
        // FSEvent attach takes a beat on macOS.
        std::thread::sleep(Duration::from_millis(150));
        std::fs::write(flows.join("a.yaml"), b"nodes: []\n").unwrap();

        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)),
            Ok(FlowsEvent::Changed)
        );
    }
}
