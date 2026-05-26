//! Track which `claude` process lives inside each daruda pane.
//!
//! Phase E uses this to:
//! - Highlight the badge for the focused tab's session in the
//!   per-lane sub-row (visual disambiguation when multiple
//!   sessions share a cwd).
//! - Drop a session from `ClaudeStatusStore` as soon as its
//!   `claude` process disappears, without waiting for `SessionEnd`
//!   or the cold-restore TTL.
//!
//! Mechanism: a background thread polls `sysinfo` every 3 s, walks
//! descendants of each registered pane's PTY shell PID, and matches
//! any process whose name or cmd contains `"claude"` against
//! `~/.claude/sessions/<pid>.json` (via [`daruda_claude::pty_link`])
//! to obtain the authoritative `session_id`.
//!
//! The result is two diff events on a `mpsc::Receiver`:
//!
//! - [`PtyTrackerEvent::BindingChanged`] — a pane's binding flipped
//!   (claude started, exited, or got swapped for a different one).
//! - [`PtyTrackerEvent::DeadSession`] — a session_id we previously
//!   reported is no longer attached to any live `claude` process,
//!   so the consumer can prune it from its store.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, SystemTime};

use daruda_claude::pty_link;

/// PaneId in the workspace layout. Mirrors `workspace::layout::PaneId`
/// but defined locally to keep `pty_tracker` independent of GPUI / the
/// workspace module.
pub type PaneId = u64;

/// Polling cadence. 5 s is a balance between responsiveness (start /
/// exit detection latency) and CPU cost of `sysinfo::refresh_processes`.
/// The refresh itself is skipped entirely when no pane is registered,
/// so this only governs the cadence of *active* tracking.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// One pane's currently-resolved `claude` process.
#[derive(Clone, Debug, PartialEq)]
pub struct PtyBinding {
    pub claude_pid: u32,
    pub session_id: String,
    /// Wall-clock at which *this tracker* first saw the binding. Not
    /// the actual process start time — sysinfo doesn't expose that
    /// portably, and our diff loop only knows what it has observed.
    /// Useful for "how long has this session been bound here?" UI.
    pub discovered_at: SystemTime,
}

/// Diff event emitted by the tracker.
#[derive(Clone, Debug)]
pub enum PtyTrackerEvent {
    /// A pane's binding changed. `binding = None` means the pane no
    /// longer has a `claude` descendant (claude exited or never
    /// started).
    BindingChanged {
        pane_id: PaneId,
        binding: Option<PtyBinding>,
    },
    /// A session_id we previously reported via `BindingChanged` no
    /// longer maps to any live `claude` process. The consumer should
    /// drop it from its store + delete the on-disk status file.
    DeadSession { session_id: String },
    /// Internal — fired once per poll so the tracker can detect a
    /// dropped receiver and exit cleanly. Consumers must filter this
    /// out before pumping events into the Workspace.
    #[doc(hidden)]
    __HeartbeatProbe,
}

/// Handle to the running tracker. Cloneable so multiple Workspace
/// entities can share the same tracker thread; the underlying poll
/// loop runs as long as at least one event receiver is connected
/// (when the last receiver drops, the next `tx.send` fails and the
/// thread exits cleanly).
#[derive(Clone)]
pub struct PtyTracker {
    inner: Arc<Mutex<TrackerInner>>,
}

#[derive(Default)]
struct TrackerInner {
    /// Registered panes — caller updates on pane create / close.
    panes: HashMap<PaneId, u32>,
    /// Last-known per-pane binding. Used to suppress duplicate
    /// `BindingChanged` events when nothing actually changed.
    bindings: HashMap<PaneId, Option<PtyBinding>>,
    /// Live session_ids reported across any pane during the most
    /// recent poll. A session_id from a previous poll missing here
    /// triggers `DeadSession`.
    last_live_sessions: HashSet<String>,
}

impl PtyTracker {
    /// Start the tracker thread. Returns the handle plus an event
    /// receiver. The thread runs until the receiver is dropped
    /// (`tx.send` fails, the loop returns).
    pub fn spawn() -> (Self, mpsc::Receiver<PtyTrackerEvent>) {
        let (tx, rx) = mpsc::channel();
        let inner = Arc::new(Mutex::new(TrackerInner::default()));
        let inner_clone = inner.clone();
        thread::spawn(move || {
            run(inner_clone, tx);
        });
        (Self { inner }, rx)
    }

    /// Register a pane's PTY shell PID so subsequent polls walk its
    /// descendants. Idempotent.
    pub fn register(&self, pane_id: PaneId, root_pid: u32) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.panes.insert(pane_id, root_pid);
        }
    }

    /// Unregister a pane (typically when the pane is closed). The
    /// next poll will not walk its tree, and any binding it had will
    /// be reported as `BindingChanged { binding: None }`.
    pub fn unregister(&self, pane_id: PaneId) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.panes.remove(&pane_id);
        }
    }
}

fn run(inner: Arc<Mutex<TrackerInner>>, tx: mpsc::Sender<PtyTrackerEvent>) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};

    let refresh_kind = ProcessRefreshKind::new()
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet);
    let mut system = System::new_with_specifics(RefreshKind::new().with_processes(refresh_kind));

    loop {
        thread::sleep(POLL_INTERVAL);
        // If the receiver is gone, no point doing the work.
        if tx.send(PtyTrackerEvent::__HeartbeatProbe).is_err() {
            return;
        }

        // Snapshot registered panes *before* the expensive refresh —
        // when no pane is registered, the entire sysinfo enumeration
        // and BFS step is skipped. macOS `sysinfo` runs
        // `sysctl(KERN_PROC)` + `proc_pidinfo` across a rayon thread
        // pool that scales linearly with system process count, so
        // this guard keeps idle CPU near zero when the user has no
        // active tab. Mutex poisoning recovery as before.
        let panes: Vec<(PaneId, u32)> = {
            let guard = match inner.lock() {
                Ok(g) => g,
                Err(poison) => {
                    inner.clear_poison();
                    poison.into_inner()
                }
            };
            guard.panes.iter().map(|(k, v)| (*k, *v)).collect()
        };

        // When no pane is registered, fall through to the bindings
        // diff with an empty new state so any lingering previous
        // bindings emit a single `BindingChanged { binding: None }`
        // and `last_live_sessions` flushes to `DeadSession` once.
        // Subsequent ticks with empty `panes` will be no-ops because
        // `bindings` and `last_live_sessions` are already empty.
        let (new_bindings, live_sessions) = if panes.is_empty() {
            (HashMap::new(), HashSet::new())
        } else {
            system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind);
            let parent_to_children = build_children_map(&system);
            let mut new_bindings: HashMap<PaneId, Option<PtyBinding>> = HashMap::new();
            let mut live_sessions: HashSet<String> = HashSet::new();
            for (pane_id, root_pid) in &panes {
                let binding = find_claude_binding(*root_pid, &system, &parent_to_children);
                if let Some(b) = &binding {
                    live_sessions.insert(b.session_id.clone());
                }
                new_bindings.insert(*pane_id, binding);
            }
            (new_bindings, live_sessions)
        };

        // Diff against last known state and emit events. Hold the lock
        // for the comparison + update so concurrent register/unregister
        // doesn't tear the bindings map.
        let dead_sessions = {
            let mut guard = match inner.lock() {
                Ok(g) => g,
                Err(poison) => {
                    inner.clear_poison();
                    poison.into_inner()
                }
            };
            for (pane_id, new) in &new_bindings {
                let prev = guard.bindings.get(pane_id);
                if prev != Some(new) {
                    let _ = tx.send(PtyTrackerEvent::BindingChanged {
                        pane_id: *pane_id,
                        binding: new.clone(),
                    });
                }
            }
            // Panes that were registered last poll but vanished now
            // (caller called `unregister`): emit BindingChanged(None)
            // so the consumer drops their tooltip / active marker.
            for (pane_id, prev) in &guard.bindings {
                if !new_bindings.contains_key(pane_id) && prev.is_some() {
                    let _ = tx.send(PtyTrackerEvent::BindingChanged {
                        pane_id: *pane_id,
                        binding: None,
                    });
                }
            }
            guard.bindings = new_bindings;

            let dead: Vec<String> = guard
                .last_live_sessions
                .difference(&live_sessions)
                .cloned()
                .collect();
            guard.last_live_sessions = live_sessions;
            dead
        };
        for session_id in dead_sessions {
            let _ = tx.send(PtyTrackerEvent::DeadSession { session_id });
        }
    }
}

/// One `parent_pid → [child_pid, …]` map computed once per poll.
fn build_children_map(system: &sysinfo::System) -> HashMap<u32, Vec<u32>> {
    let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
    for (pid, proc) in system.processes() {
        if let Some(parent) = proc.parent() {
            map.entry(parent.as_u32()).or_default().push(pid.as_u32());
        }
    }
    map
}

/// BFS from `root_pid` for the first descendant whose name or cmd
/// looks like `claude`, then look up its `session_id` via pty_link.
fn find_claude_binding(
    root_pid: u32,
    system: &sysinfo::System,
    children: &HashMap<u32, Vec<u32>>,
) -> Option<PtyBinding> {
    let mut queue: VecDeque<u32> = VecDeque::new();
    queue.push_back(root_pid);
    let mut seen: HashSet<u32> = HashSet::new();
    seen.insert(root_pid);

    while let Some(pid) = queue.pop_front() {
        if pid != root_pid
            && is_claude_process(pid, system)
            && let Some(meta) = pty_link::read_session_meta(pid)
        {
            return Some(PtyBinding {
                claude_pid: pid,
                session_id: meta.session_id,
                discovered_at: SystemTime::now(),
            });
        }
        if let Some(kids) = children.get(&pid) {
            for kid in kids {
                if seen.insert(*kid) {
                    queue.push_back(*kid);
                }
            }
        }
    }
    None
}

/// Heuristic — is this process a Claude Code session? Matches the
/// binary name (`claude`) or the first cmd argument (npm-installed
/// builds run as `node /path/to/claude`).
fn is_claude_process(pid: u32, system: &sysinfo::System) -> bool {
    let Some(proc) = system.process(sysinfo::Pid::from_u32(pid)) else {
        return false;
    };
    let name = proc.name().to_string_lossy();
    if name == "claude" {
        return true;
    }
    // npm install path: `node /Users/.../node_modules/.bin/claude ...`
    proc.cmd()
        .iter()
        .any(|arg| arg.to_string_lossy().contains("claude"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_unregister_round_trip() {
        let (tracker, _rx) = PtyTracker::spawn();
        tracker.register(1, 1234);
        tracker.register(2, 5678);
        {
            let inner = tracker.inner.lock().unwrap();
            assert_eq!(inner.panes.len(), 2);
            assert_eq!(inner.panes.get(&1), Some(&1234));
        }
        tracker.unregister(1);
        {
            let inner = tracker.inner.lock().unwrap();
            assert_eq!(inner.panes.len(), 1);
            assert!(!inner.panes.contains_key(&1));
        }
    }
}
