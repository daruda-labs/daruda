//! MCP filesystem watcher — recursive `notify` subscriptions on the
//! parent directories of `~/.claude/settings.json` (personal scope)
//! and `<lane>/.mcp.json` (project scope).
//!
//! Mirrors `hooks::skills_watcher`'s 2-thread layout:
//! 1. **FSEvent thread** — owns the `notify::Watcher`, blocks on the
//!    shutdown channel, drops the watcher when the caller releases the
//!    handle.
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

use crate::agent::mcp::McpScope;

/// Coalescing window. Same value as Skills — atomic-rename bursts
/// finish within this much, the panel still reads as live.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// One coalesced reload signal per scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpEvent {
    Reloaded(McpScope),
}

/// Caller-side handle. Dropping this closes both watcher threads.
pub struct McpWatcherHandle {
    pub(super) _shutdown_tx: mpsc::Sender<()>,
}

/// Spawn the watcher.
///
/// `project_path` is the active lane's `.mcp.json`. `None` when no
/// lane is active (welcome-style window) — only the personal scope
/// then has a subscription.
///
/// Returns `(events, handle)`: the pump task takes ownership of
/// `events`, while the handle stays on `Workspace` so dropping it
/// stops the worker threads.
pub fn spawn(
    project_path: Option<PathBuf>,
    personal_path: PathBuf,
) -> (mpsc::Receiver<McpEvent>, McpWatcherHandle) {
    use notify::{RecursiveMode, Watcher};

    let (raw_tx, raw_rx) = mpsc::channel::<McpScope>();
    let (event_tx, event_rx) = mpsc::channel::<McpEvent>();
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

    // Resolve canonical anchors for `starts_with` comparisons. We watch
    // the parent directory so we still see file *creation* events.
    let project_anchor = project_path
        .as_deref()
        .and_then(|p| nearest_existing_ancestor_for_file(p, 2));
    let personal_anchor = nearest_existing_ancestor_for_file(&personal_path, 2);

    let project_match = project_path
        .as_deref()
        .map(|p| canonical_match_for_target(p, project_anchor.as_deref()));
    let personal_match = canonical_match_for_target(&personal_path, personal_anchor.as_deref());

    std::thread::spawn(move || {
        let raw_tx_inner = raw_tx.clone();
        let project_match_clone = project_match.clone();
        let personal_match_clone = personal_match.clone();

        let mut watcher =
            match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                let Ok(event) = res else {
                    return;
                };
                for path in &event.paths {
                    let scope = if let Some(proj) = &project_match_clone
                        && path == proj
                    {
                        // Project file is a single path — exact match
                        // (or the parent directory, which we ignore).
                        McpScope::Project
                    } else if path == &personal_match_clone {
                        McpScope::Personal
                    } else {
                        continue;
                    };
                    let _ = raw_tx_inner.send(scope);
                }
            }) {
                Ok(w) => w,
                Err(_) => return,
            };

        // Watch the *parent* directory of each target path so we see
        // create events for files that didn't exist at spawn time.
        // RecursiveMode::NonRecursive is enough — every relevant event
        // lands directly in the parent directory.
        if let Some(anchor) = project_anchor.as_deref() {
            let _ = watcher.watch(anchor, RecursiveMode::NonRecursive);
        }
        if let Some(anchor) = personal_anchor.as_deref() {
            let already_covered = project_anchor.as_deref().is_some_and(|p| anchor == p);
            if !already_covered {
                let _ = watcher.watch(anchor, RecursiveMode::NonRecursive);
            }
        }

        let _ = shutdown_rx.recv();
        drop(raw_tx);
    });

    // Debounce thread.
    std::thread::spawn(move || {
        while let Ok(first_scope) = raw_rx.recv() {
            let mut pending = ScopeFlags::default();
            pending.set(first_scope);

            let deadline = std::time::Instant::now() + DEBOUNCE;
            loop {
                let now = std::time::Instant::now();
                if now >= deadline {
                    break;
                }
                match raw_rx.recv_timeout(deadline - now) {
                    Ok(scope) => pending.set(scope),
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

    (
        event_rx,
        McpWatcherHandle {
            _shutdown_tx: shutdown_tx,
        },
    )
}

#[derive(Clone, Copy, Default)]
struct ScopeFlags {
    project: bool,
    personal: bool,
}

impl ScopeFlags {
    fn set(&mut self, scope: McpScope) {
        match scope {
            McpScope::Project => self.project = true,
            McpScope::Personal => self.personal = true,
        }
    }
}

fn flush(pending: &ScopeFlags, tx: &mpsc::Sender<McpEvent>) -> bool {
    if pending.project && tx.send(McpEvent::Reloaded(McpScope::Project)).is_err() {
        return false;
    }
    if pending.personal && tx.send(McpEvent::Reloaded(McpScope::Personal)).is_err() {
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
/// Fresh-install caveat: when `~/.claude/` doesn't exist yet, the
/// personal anchor walks up to `~`. The watcher then receives events
/// for every file change in `~`, but the per-event `path == match`
/// filter accepts only the exact target path, so noise is rejected.
/// `refresh_mcp_watcher` is called on every lane swap — once a
/// save has created `~/.claude/settings.json`, the next refresh
/// re-anchors the watcher onto the now-existing parent directory.
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

    /// Smoke test — write to the personal file and confirm a Reload
    /// event lands in the receiver. Skipped on non-macOS for brevity.
    #[test]
    #[ignore = "FSEvents-dependent — run via `cargo test --ignored`"]
    fn personal_change_triggers_reload() {
        let _g = crate::hooks::tests_common::fsevent_serial();
        let dir = tempfile::tempdir().unwrap();
        let personal = dir.path().join("settings.json");
        std::fs::write(&personal, b"{}\n").unwrap();

        let (rx, _handle) = spawn(None, personal.clone());

        // FSEvent attach + canonicalize takes a beat on macOS.
        std::thread::sleep(Duration::from_millis(150));
        std::fs::write(&personal, b"{\"mcpServers\":{}}\n").unwrap();

        let evt = rx.recv_timeout(Duration::from_secs(2));
        assert!(evt.is_ok(), "expected Reload event, got {evt:?}");
    }
}
