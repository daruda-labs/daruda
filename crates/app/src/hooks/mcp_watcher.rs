//! MCP filesystem watcher — `notify` subscriptions on the parent
//! directories of `~/.claude.json` (User + Local scopes) and
//! `<lane>/.mcp.json` (Project scope).
//!
//! Layout:
//! 1. **`dir_watch`** — owns the `notify::Watcher` (via the [`McpWatcherHandle`]'s
//!    [`crate::dir_watch::DirWatcher`]); classifies events to a [`WatchedFile`]
//!    and recovers FSEvents drops (sleep/wake) by reloading every scope.
//! 2. **Debounce thread** — collapses event bursts (atomic-rename
//!    saves emit ≥3 events per file) into one [`McpEvent`] per scope
//!    per [`DEBOUNCE`] window.
//!
//! macOS quirks (same as Skills):
//! - `/var/folders → /private/var/folders` symlink — canonicalize the
//!   anchor before comparing event paths.
//! - The target file may not exist yet — fall back to watching the
//!   nearest existing ancestor, then filter via `starts_with`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

/// Coalescing window. Same value as Skills — atomic-rename bursts
/// finish within this much, the panel still reads as live.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// One coalesced reload signal per physical config file. User and
/// Local scopes share `~/.claude.json`, so a single
/// [`McpEvent::ClaudeJsonReloaded`] reloads both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpEvent {
    /// `~/.claude.json` changed — reload User + Local scopes.
    ClaudeJsonReloaded,
    /// `<lane>/.mcp.json` changed — reload the Project scope.
    ProjectReloaded,
}

/// Which physical file a raw watcher event maps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WatchedFile {
    ClaudeJson,
    Project,
}

/// Caller-side handle. Dropping it drops the [`crate::dir_watch::DirWatcher`]
/// (stops the watch), which disconnects the raw channel and ends the debounce
/// thread. Re-spawned on lane / cwd changes — dropping the old handle releases
/// the old watcher, so re-anchoring never leaks.
pub struct McpWatcherHandle {
    pub(super) _watcher: crate::dir_watch::DirWatcher,
}

/// Spawn the watcher.
///
/// `project_paths` are the active lane's `.mcp.json` candidates — the
/// lane root and the focused terminal's cwd (when it differs). Empty
/// when no lane is active (welcome-style window) — only `~/.claude.json`
/// then has a subscription.
///
/// `claude_json_path` is `~/.claude.json` (User + Local scopes). Its
/// parent is the home directory, which is busy — the per-event exact
/// path filter rejects every change that isn't `~/.claude.json` itself.
///
/// Returns `(events, handle)`: the pump task takes ownership of
/// `events`, while the handle stays on `Workspace` so dropping it
/// stops the worker threads.
pub fn spawn(
    project_paths: Vec<PathBuf>,
    claude_json_path: PathBuf,
) -> (mpsc::Receiver<McpEvent>, McpWatcherHandle) {
    use notify::RecursiveMode;

    let (event_tx, event_rx) = mpsc::channel::<McpEvent>();

    // Resolve canonical anchors for `==` comparisons. We watch the parent
    // directory so we still see file *creation* events. The Project scope can
    // have two `.mcp.json` targets (lane root + the focused terminal's cwd),
    // so anchors/matches are sets.
    let mut project_anchors: Vec<PathBuf> = Vec::new();
    let mut project_matches: Vec<PathBuf> = Vec::new();
    for p in &project_paths {
        let anchor = nearest_existing_ancestor_for_file(p, 2);
        project_matches.push(canonical_match_for_target(p, anchor.as_deref()));
        if let Some(a) = anchor
            && !project_anchors.contains(&a)
        {
            project_anchors.push(a);
        }
    }
    let claude_json_anchor = nearest_existing_ancestor_for_file(&claude_json_path, 2);
    let claude_json_match =
        canonical_match_for_target(&claude_json_path, claude_json_anchor.as_deref());
    let has_claude_json = claude_json_anchor.is_some();

    // Watch the *parent* directory of each target so create events for
    // not-yet-existing files are seen. NonRecursive: every relevant event
    // lands directly in the parent.
    let mut anchors = project_anchors;
    if let Some(a) = claude_json_anchor
        && !anchors.contains(&a)
    {
        anchors.push(a);
    }

    let has_project = !project_matches.is_empty();
    let classify = move |event: &notify::Event| {
        let mut out = Vec::new();
        for path in &event.paths {
            if project_matches.iter().any(|m| m == path) {
                out.push(WatchedFile::Project);
            } else if *path == claude_json_match {
                out.push(WatchedFile::ClaudeJson);
            }
        }
        out
    };
    // The scopes this watcher actually covers — the single source for the
    // rescan fan-out, derived from the same anchors that gate watching (so a
    // scope whose target dir doesn't exist isn't spuriously reloaded). An
    // FSEvents drop (sleep/wake) can't say which file changed, so reload every
    // covered scope. Keep in lockstep with `classify`: any scope it can emit
    // must be reloadable here.
    let mut watched = Vec::new();
    if has_claude_json {
        watched.push(WatchedFile::ClaudeJson);
    }
    if has_project {
        watched.push(WatchedFile::Project);
    }
    let rescan = move || watched.clone();

    let (raw_rx, watcher) =
        crate::dir_watch::spawn_dir_watcher(&anchors, RecursiveMode::NonRecursive, classify, rescan);

    // Debounce thread.
    std::thread::spawn(move || {
        while let Ok(first) = raw_rx.recv() {
            let mut pending = FileFlags::default();
            pending.set(first);

            let deadline = std::time::Instant::now() + DEBOUNCE;
            loop {
                let now = std::time::Instant::now();
                if now >= deadline {
                    break;
                }
                match raw_rx.recv_timeout(deadline - now) {
                    Ok(which) => pending.set(which),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => {
                        flush(&pending, &event_tx);
                        return;
                    }
                }
            }
            if !flush(&pending, &event_tx) {
                break;
            }
        }
    });

    (event_rx, McpWatcherHandle { _watcher: watcher })
}

#[derive(Clone, Copy, Default)]
struct FileFlags {
    project: bool,
    claude_json: bool,
}

impl FileFlags {
    fn set(&mut self, which: WatchedFile) {
        match which {
            WatchedFile::Project => self.project = true,
            WatchedFile::ClaudeJson => self.claude_json = true,
        }
    }
}

fn flush(pending: &FileFlags, tx: &mpsc::Sender<McpEvent>) -> bool {
    if pending.project && tx.send(McpEvent::ProjectReloaded).is_err() {
        return false;
    }
    if pending.claude_json && tx.send(McpEvent::ClaudeJsonReloaded).is_err() {
        return false;
    }
    true
}

fn canonicalize_or_self(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Walk up at most `max_ascend` levels from `target` until we find a
/// directory that exists. `target` is a file path here — we skip the
/// file itself and start from its parent, then ascend further until
/// something exists.
///
/// `~/.claude.json`'s parent is the home directory, which always
/// exists, so its anchor is `~` directly. The watcher then receives
/// events for every direct child of `~`, but the per-event
/// `path == match` filter accepts only the exact target path, so noise
/// is rejected. For a project `.mcp.json` whose parent doesn't exist
/// yet (fresh worktree), the walk ascends until something exists;
/// `refresh_mcp_watcher` re-anchors on the next lane swap once the
/// directory is created.
fn nearest_existing_ancestor_for_file(target: &Path, max_ascend: usize) -> Option<PathBuf> {
    let mut current = target.parent()?.to_path_buf();
    let mut hops = 0usize;
    loop {
        if current.exists() {
            return Some(current);
        }
        if hops >= max_ascend {
            return None;
        }
        match current.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => {
                current = parent.to_path_buf();
                hops += 1;
            }
            _ => return None,
        }
    }
}

/// Build the canonical match path for the target file. The anchor is
/// canonicalized (handles `/var → /private/var`); the file's tail
/// component is appended verbatim. The renderer compares event paths
/// against this with `==` (single file, no descendants).
fn canonical_match_for_target(target: &Path, existing_anchor: Option<&Path>) -> PathBuf {
    if let Some(anchor) = existing_anchor {
        let canonical_anchor = canonicalize_or_self(anchor);
        match target.strip_prefix(anchor) {
            Ok(tail) if !tail.as_os_str().is_empty() => canonical_anchor.join(tail),
            _ => canonical_anchor,
        }
    } else {
        canonicalize_or_self(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Smoke test — write to `~/.claude.json` and confirm a
    /// `ClaudeJsonReloaded` event lands in the receiver. Skipped on
    /// non-macOS for brevity.
    #[test]
    #[ignore = "FSEvents-dependent — run via `cargo test --ignored`"]
    fn claude_json_change_triggers_reload() {
        let _g = crate::hooks::tests_common::fsevent_serial();
        let dir = tempfile::tempdir().unwrap();
        let claude_json = dir.path().join(".claude.json");
        std::fs::write(&claude_json, b"{}\n").unwrap();

        let (rx, _handle) = spawn(Vec::new(), claude_json.clone());

        // FSEvent attach + canonicalize takes a beat on macOS.
        std::thread::sleep(Duration::from_millis(150));
        std::fs::write(&claude_json, b"{\"mcpServers\":{}}\n").unwrap();

        let evt = rx.recv_timeout(Duration::from_secs(2));
        assert_eq!(evt, Ok(McpEvent::ClaudeJsonReloaded));
    }
}
