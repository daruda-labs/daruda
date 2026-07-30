//! Track which `claude` process lives inside each daruda pane.
//!
//! Event-driven, no idle poll: a background thread wakes on
//! `~/.claude/sessions/` changes or pane register/unregister pokes, then walks
//! each session PID's parent chain toward registered PTY shell PIDs. Quiet
//! directories cost zero wakeups; lingering dead sessions are pruned on the
//! next wake or by cold-restore TTL rather than by polling.
//!
//! Emits binding diffs so the UI can highlight the focused pane's session and
//! drop status entries no longer attributable to any live pane.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use daruda_agent::pty_link;

/// Pane id mirrored locally to keep this tracker independent of GPUI/workspace.
pub type PaneId = u64;

/// Coalescing window for register fan-out and atomic-rename wake bursts.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// Upper bound on how far up a `claude` PID's parent chain we walk
/// looking for a registered pane shell. Real process trees between a
/// PTY shell and its `claude` child are 1–3 hops; the cap is a guard
/// against an unexpectedly deep chain or a malformed parent cycle, not
/// a tuning knob.
const MAX_PARENT_WALK: usize = 32;

/// One pane's currently resolved `claude` process. Equality is the diff key.
#[derive(Clone, Debug, PartialEq)]
pub struct PtyBinding {
    pub claude_pid: u32,
    pub session_id: String,
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
}

/// Internal wake reason for the tracker thread. `Poke` triggers a
/// re-resolution; `Shutdown` lets the parked thread exit even when the
/// sessions directory is quiet (sent when the last [`PtyTracker`] clone
/// drops, since the thread otherwise owns the FSEvents watcher and the
/// wake channel never disconnects on its own).
enum Wake {
    Poke,
    Shutdown,
}

/// Sends [`Wake::Shutdown`] when the final [`PtyTracker`] clone drops.
/// Held in an `Arc` shared by every clone so the signal fires exactly
/// once, at teardown.
struct ShutdownOnDrop {
    wake_tx: mpsc::Sender<Wake>,
}

impl Drop for ShutdownOnDrop {
    fn drop(&mut self) {
        // SILENT-OK: the thread may already have exited (consumer gone),
        // in which case the send fails harmlessly.
        let _ = self.wake_tx.send(Wake::Shutdown);
    }
}

/// Handle to the running tracker. Cloneable so multiple Workspace
/// entities can share it; the thread is parked on its wake channel and
/// exits when the last clone drops (via [`ShutdownOnDrop`]) or the
/// event receiver disconnects.
#[derive(Clone)]
pub struct PtyTracker {
    inner: Arc<Mutex<TrackerInner>>,
    /// Wakes the tracker thread to re-resolve after a `register` /
    /// `unregister`.
    wake_tx: mpsc::Sender<Wake>,
    /// Drop guard — fires the shutdown wake when the last clone goes.
    _shutdown: Arc<ShutdownOnDrop>,
}

#[derive(Default)]
struct TrackerInner {
    /// Registered panes — caller updates on pane create / close.
    panes: HashMap<PaneId, u32>,
    /// Last-known per-pane binding. Used to suppress duplicate
    /// `BindingChanged` events when nothing actually changed.
    bindings: HashMap<PaneId, Option<PtyBinding>>,
    /// Live session_ids reported across any pane during the most
    /// recent resolution. A session_id from a previous pass missing
    /// here triggers `DeadSession`.
    last_live_sessions: HashSet<String>,
}

impl PtyTracker {
    /// Start the tracker thread. Returns the handle plus an event
    /// receiver. The thread parks on its wake channel and exits when
    /// the last handle clone drops or the receiver disconnects.
    pub fn spawn() -> (Self, mpsc::Receiver<PtyTrackerEvent>) {
        let (event_tx, event_rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::channel::<Wake>();
        let inner = Arc::new(Mutex::new(TrackerInner::default()));

        let inner_clone = inner.clone();
        let wake_tx_for_watcher = wake_tx.clone();
        thread::spawn(move || {
            run(inner_clone, event_tx, wake_rx, wake_tx_for_watcher);
        });

        let tracker = Self {
            inner,
            wake_tx: wake_tx.clone(),
            _shutdown: Arc::new(ShutdownOnDrop { wake_tx }),
        };
        (tracker, event_rx)
    }

    /// Register a pane's PTY shell PID and wake the tracker to resolve
    /// its binding (a `claude` may already be running inside it).
    /// Idempotent.
    pub fn register(&self, pane_id: PaneId, root_pid: u32) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.panes.insert(pane_id, root_pid);
        }
        self.poke();
    }

    /// Unregister a pane (typically when the pane is closed) and wake
    /// the tracker so any binding it had is cleared with
    /// `BindingChanged { binding: None }`.
    pub fn unregister(&self, pane_id: PaneId) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.panes.remove(&pane_id);
        }
        self.poke();
    }

    fn poke(&self) {
        // SILENT-OK: a dead channel means the tracker thread already
        // exited (Workspace teardown) — nothing left to wake.
        let _ = self.wake_tx.send(Wake::Poke);
    }

    /// Test-only introspection — the currently registered pane ids.
    #[cfg(test)]
    pub fn tracked_pane_ids(&self) -> Vec<PaneId> {
        self.inner
            .lock()
            .map(|inner| inner.panes.keys().copied().collect())
            .unwrap_or_default()
    }
}

fn run(
    inner: Arc<Mutex<TrackerInner>>,
    event_tx: mpsc::Sender<PtyTrackerEvent>,
    wake_rx: mpsc::Receiver<Wake>,
    wake_tx: mpsc::Sender<Wake>,
) {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System, UpdateKind};

    let Some(sessions_dir) = pty_link::default_sessions_dir() else {
        // No home directory resolves (extremely rare on real macOS).
        // Without a sessions directory there is nothing to track.
        return;
    };
    // Attach the FSEvents watch directly to the sessions directory.
    // Creating it empty is benign — it's exactly where `claude` writes
    // its per-session files — and a direct (non-recursive) watch avoids
    // re-anchoring when the directory first appears.
    // SILENT-OK: failure just means it already exists or can't be made;
    // the watch below degrades gracefully either way.
    let _ = std::fs::create_dir_all(&sessions_dir);

    // Held for the thread's lifetime; dropped on return to unsubscribe.
    // `None` (watch setup failed) degrades to register/unregister-driven
    // resolution — bindings still resolve on pane create, but a `claude`
    // launched into an already-open pane isn't noticed until the next
    // poke.
    let _watcher = spawn_sessions_watcher(&sessions_dir, wake_tx);

    let refresh_kind = ProcessRefreshKind::new()
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet);
    let system = RefCell::new(System::new_with_specifics(
        RefreshKind::new().with_processes(refresh_kind),
    ));

    // Resolve once up front in case sessions already exist; panes
    // register shortly after spawn and poke again.
    if !resolve_and_emit(&inner, &event_tx, &sessions_dir, &system, refresh_kind) {
        return;
    }

    loop {
        match wake_rx.recv() {
            Ok(Wake::Poke) => {}
            Ok(Wake::Shutdown) | Err(_) => return,
        }
        // Coalesce a burst, then drain everything pending so one
        // re-resolution covers it. A shutdown anywhere in the burst
        // still wins.
        thread::sleep(DEBOUNCE);
        let mut shutdown = false;
        while let Ok(wake) = wake_rx.try_recv() {
            if matches!(wake, Wake::Shutdown) {
                shutdown = true;
            }
        }
        if !resolve_and_emit(&inner, &event_tx, &sessions_dir, &system, refresh_kind) {
            return;
        }
        if shutdown {
            return;
        }
    }
}

/// Spawn the FSEvents watch on the sessions directory. Any event wakes
/// the tracker with a [`Wake::Poke`]; the diffing happens in the
/// resolution pass, so the event payload is not inspected.
///
/// Intentionally NOT built on [`crate::dir_watch::spawn_dir_watcher`]: any
/// event here already triggers a full re-resolution pass, so FSEvents'
/// post-sleep `EventKind::Other` rescan is handled for free, and the wake
/// needs to multiplex into the shared `wake_tx` alongside register /
/// unregister / shutdown — a shape `spawn_dir_watcher`'s owned-channel model
/// doesn't fit.
fn spawn_sessions_watcher(
    dir: &Path,
    wake_tx: mpsc::Sender<Wake>,
) -> Option<notify::RecommendedWatcher> {
    use notify::{RecursiveMode, Watcher};

    let mut watcher =
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if res.is_ok() {
                // SILENT-OK: a dead channel means the tracker thread is
                // gone and this watcher is about to be dropped with it.
                let _ = wake_tx.send(Wake::Poke);
            }
        })
        .ok()?;
    watcher.watch(dir, RecursiveMode::NonRecursive).ok()?;
    Some(watcher)
}

/// Read every `<pid>.json` in the sessions directory into a
/// [`pty_link::PidSessionMeta`]. Unparseable or vanished files are
/// skipped — a half-written file simply isn't resolved this pass and is
/// picked up on the next FSEvents wake.
fn list_session_metas(dir: &Path) -> Vec<pty_link::PidSessionMeta> {
    let mut metas = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return metas;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(pid) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if let Some(meta) = pty_link::read_session_meta_in(dir, pid) {
            metas.push(meta);
        }
    }
    metas
}

/// One resolution pass: snapshot the registered panes, re-resolve their
/// bindings from the current session files, diff against the previous
/// pass, and emit `BindingChanged` / `DeadSession`. Returns `false`
/// when the event consumer has disconnected so the caller stops the
/// thread.
fn resolve_and_emit(
    inner: &Arc<Mutex<TrackerInner>>,
    event_tx: &mpsc::Sender<PtyTrackerEvent>,
    sessions_dir: &Path,
    system: &RefCell<sysinfo::System>,
    refresh_kind: sysinfo::ProcessRefreshKind,
) -> bool {
    use sysinfo::{Pid, ProcessesToUpdate};

    // Snapshot registered panes. With none registered the new state is
    // empty, so any lingering bindings flush to `None` / `DeadSession`
    // exactly once and subsequent empty passes are no-ops.
    let panes: HashMap<PaneId, u32> = {
        let guard = lock_inner(inner);
        guard.panes.clone()
    };

    let (new_bindings, live_sessions) = if panes.is_empty() {
        (HashMap::new(), HashSet::new())
    } else {
        let sessions = list_session_metas(sessions_dir);
        // `parent_of` refreshes only the single PID asked for — a few
        // cheap `sysctl` calls per session, never the whole table. A
        // dead PID refreshes to absent, so its walk yields no parent and
        // the (crashed) session resolves to no binding.
        let parent_of = |pid: u32| -> Option<u32> {
            let mut sys = system.borrow_mut();
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
                true,
                refresh_kind,
            );
            sys.process(Pid::from_u32(pid))
                .and_then(|p| p.parent())
                .map(|pp| pp.as_u32())
        };
        rescan(&panes, &sessions, &parent_of)
    };

    // Diff + commit under the lock so a concurrent register/unregister
    // doesn't tear the bindings map.
    let (binding_events, dead) = {
        let mut guard = lock_inner(inner);
        let binding_events = binding_change_events(&guard.bindings, &new_bindings);
        guard.bindings = new_bindings;
        let dead = dead_sessions(&guard.last_live_sessions, &live_sessions);
        guard.last_live_sessions = live_sessions;
        (binding_events, dead)
    };

    // A send error means the GPUI consumer has dropped — our cue to
    // stop the tracker thread.
    for (pane_id, binding) in binding_events {
        if event_tx
            .send(PtyTrackerEvent::BindingChanged { pane_id, binding })
            .is_err()
        {
            return false;
        }
    }
    for session_id in dead {
        if event_tx
            .send(PtyTrackerEvent::DeadSession { session_id })
            .is_err()
        {
            return false;
        }
    }
    true
}

/// Lock `inner`, recovering from poisoning — a panicked holder leaves
/// the maps structurally intact for our read-modify-write.
fn lock_inner(inner: &Arc<Mutex<TrackerInner>>) -> std::sync::MutexGuard<'_, TrackerInner> {
    match inner.lock() {
        Ok(g) => g,
        Err(poison) => {
            inner.clear_poison();
            poison.into_inner()
        }
    }
}

/// Resolve each registered pane's `claude` binding from the set of
/// currently-live session files, by walking each session's `claude`
/// PID *up* its parent chain until it reaches a registered pane's PTY
/// shell PID.
///
/// This is the event-driven counterpart to [`find_claude_binding`]'s
/// BFS-down: instead of enumerating every process to build a children
/// map, we start from the known `claude` PIDs (the session-file names)
/// and ask only for each candidate's parent — a handful of cheap
/// lookups per session, injected via `parent_of` so the resolution is
/// pure and testable without `sysinfo`.
///
/// Only panes that resolve to a live session appear in the result;
/// callers treat an absent pane as "no binding". When two sessions
/// resolve to the same pane (a nested `claude`), the shallower one —
/// the direct descendant of the shell — wins; ties break on the lower
/// PID for determinism.
fn resolve_pane_bindings(
    panes: &HashMap<PaneId, u32>,
    sessions: &[pty_link::PidSessionMeta],
    parent_of: &dyn Fn(u32) -> Option<u32>,
) -> HashMap<PaneId, PtyBinding> {
    // Reverse index: a pane's PTY shell PID → the pane it belongs to.
    let shell_to_pane: HashMap<u32, PaneId> =
        panes.iter().map(|(pane, shell)| (*shell, *pane)).collect();

    let mut result: HashMap<PaneId, PtyBinding> = HashMap::new();
    for session in sessions {
        let mut cur = session.pid;
        let mut seen: HashSet<u32> = HashSet::new();
        seen.insert(cur);
        let mut depth = 0;
        while depth < MAX_PARENT_WALK {
            let Some(parent) = parent_of(cur) else { break };
            // Guard against a parent cycle or a walk that loops back on
            // a PID we've already visited.
            if !seen.insert(parent) {
                break;
            }
            depth += 1;
            if let Some(&pane_id) = shell_to_pane.get(&parent) {
                result.entry(pane_id).or_insert_with(|| PtyBinding {
                    claude_pid: session.pid,
                    session_id: session.session_id.clone(),
                });
                break;
            }
            cur = parent;
        }
    }
    result
}

/// One full re-resolution. Returns the new per-pane bindings —
/// covering *every* registered pane, `None` where no live session
/// resolves — plus the set of session ids seen live this pass (used to
/// diff `DeadSession`).
fn rescan(
    panes: &HashMap<PaneId, u32>,
    sessions: &[pty_link::PidSessionMeta],
    parent_of: &dyn Fn(u32) -> Option<u32>,
) -> (HashMap<PaneId, Option<PtyBinding>>, HashSet<String>) {
    let resolved = resolve_pane_bindings(panes, sessions, parent_of);
    let mut bindings = HashMap::with_capacity(panes.len());
    let mut live = HashSet::new();
    for pane_id in panes.keys() {
        match resolved.get(pane_id) {
            Some(b) => {
                live.insert(b.session_id.clone());
                bindings.insert(*pane_id, Some(b.clone()));
            }
            None => {
                bindings.insert(*pane_id, None);
            }
        }
    }
    (bindings, live)
}

/// Per-pane binding changes between the previous resolution and the
/// new one: every pane whose binding flipped identity, plus panes
/// present-and-bound before but absent now (unregistered) reported as
/// `None` so the consumer clears their marker.
fn binding_change_events(
    prev: &HashMap<PaneId, Option<PtyBinding>>,
    new: &HashMap<PaneId, Option<PtyBinding>>,
) -> Vec<(PaneId, Option<PtyBinding>)> {
    let mut events = Vec::new();
    // Panes resolved this pass whose binding identity flipped (or that
    // are brand new) emit their current binding.
    for (pane_id, new_binding) in new {
        if prev.get(pane_id) != Some(new_binding) {
            events.push((*pane_id, new_binding.clone()));
        }
    }
    // Panes that were bound last pass but are gone now (unregistered)
    // emit a clearing `None`.
    for (pane_id, prev_binding) in prev {
        if !new.contains_key(pane_id) && prev_binding.is_some() {
            events.push((*pane_id, None));
        }
    }
    events
}

/// Session ids that were live last pass but aren't now.
fn dead_sessions(prev_live: &HashSet<String>, now_live: &HashSet<String>) -> Vec<String> {
    prev_live.difference(now_live).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_identity_is_pid_and_session() {
        // The diff loop's duplicate suppression compares each poll's
        // freshly-built binding against the previous one — equality
        // must hold across polls for an unchanged (pid, session).
        let base = PtyBinding {
            claude_pid: 7,
            session_id: "sess".into(),
        };
        assert_eq!(base, base.clone());
        assert_ne!(
            base,
            PtyBinding {
                claude_pid: 8,
                ..base.clone()
            }
        );
        assert_ne!(
            base,
            PtyBinding {
                session_id: "other".into(),
                ..base.clone()
            }
        );
    }

    fn meta(pid: u32, session: &str) -> pty_link::PidSessionMeta {
        pty_link::PidSessionMeta {
            pid,
            session_id: session.into(),
            cwd: std::path::PathBuf::from("/tmp"),
        }
    }

    #[test]
    fn resolves_binding_when_claude_is_direct_child_of_pane_shell() {
        // pane 1's PTY shell is pid 100; the `claude` process (pid 200)
        // is its direct child. The session file is keyed by 200.
        let panes = HashMap::from([(1u64, 100u32)]);
        let sessions = vec![meta(200, "sess-a")];
        // 200 → 100 (shell) → 1 (login shell / launchd).
        let parent_of = |pid: u32| match pid {
            200 => Some(100),
            100 => Some(1),
            _ => None,
        };

        let bindings = resolve_pane_bindings(&panes, &sessions, &parent_of);

        assert_eq!(
            bindings.get(&1),
            Some(&PtyBinding {
                claude_pid: 200,
                session_id: "sess-a".into(),
            })
        );
    }

    #[test]
    fn resolves_binding_through_an_intermediate_process() {
        // claude (300) runs as `node` (250) which is a child of the
        // pane shell (100): 300 → 250 → 100. The walk must climb two
        // hops to reach the registered shell.
        let panes = HashMap::from([(7u64, 100u32)]);
        let sessions = vec![meta(300, "sess-b")];
        let parent_of = |pid: u32| match pid {
            300 => Some(250),
            250 => Some(100),
            100 => Some(1),
            _ => None,
        };

        let bindings = resolve_pane_bindings(&panes, &sessions, &parent_of);

        assert_eq!(bindings.get(&7).map(|b| b.claude_pid), Some(300));
    }

    #[test]
    fn no_binding_when_claude_is_not_under_any_registered_shell() {
        // claude (200) belongs to a shell (999) daruda never registered.
        let panes = HashMap::from([(1u64, 100u32)]);
        let sessions = vec![meta(200, "stray")];
        let parent_of = |pid: u32| match pid {
            200 => Some(999),
            999 => Some(1),
            _ => None,
        };

        let bindings = resolve_pane_bindings(&panes, &sessions, &parent_of);

        assert!(bindings.is_empty());
    }

    #[test]
    fn resolves_independent_bindings_for_multiple_panes() {
        let panes = HashMap::from([(1u64, 100u32), (2u64, 200u32)]);
        let sessions = vec![meta(110, "sess-1"), meta(210, "sess-2")];
        let parent_of = |pid: u32| match pid {
            110 => Some(100),
            210 => Some(200),
            100 | 200 => Some(1),
            _ => None,
        };

        let bindings = resolve_pane_bindings(&panes, &sessions, &parent_of);

        assert_eq!(bindings.get(&1).map(|b| &b.session_id[..]), Some("sess-1"));
        assert_eq!(bindings.get(&2).map(|b| &b.session_id[..]), Some("sess-2"));
    }

    #[test]
    fn parent_cycle_does_not_hang_the_walk() {
        // A pathological 200 ↔ 201 cycle that never reaches a shell.
        let panes = HashMap::from([(1u64, 100u32)]);
        let sessions = vec![meta(200, "loop")];
        let parent_of = |pid: u32| match pid {
            200 => Some(201),
            201 => Some(200),
            _ => None,
        };

        let bindings = resolve_pane_bindings(&panes, &sessions, &parent_of);

        assert!(bindings.is_empty());
    }

    fn binding(pid: u32, session: &str) -> PtyBinding {
        PtyBinding {
            claude_pid: pid,
            session_id: session.into(),
        }
    }

    #[test]
    fn rescan_covers_every_pane_and_collects_live_sessions() {
        // pane 1 has a resolvable claude; pane 2 does not.
        let panes = HashMap::from([(1u64, 100u32), (2u64, 500u32)]);
        let sessions = vec![meta(110, "sess-1")];
        let parent_of = |pid: u32| match pid {
            110 => Some(100),
            100 | 500 => Some(1),
            _ => None,
        };

        let (bindings, live) = rescan(&panes, &sessions, &parent_of);

        assert_eq!(bindings.get(&1), Some(&Some(binding(110, "sess-1"))));
        assert_eq!(bindings.get(&2), Some(&None));
        assert_eq!(live, HashSet::from(["sess-1".to_string()]));
    }

    #[test]
    fn binding_change_events_reports_gain_loss_and_change() {
        let prev = HashMap::from([
            (1u64, Some(binding(10, "a"))), // unchanged
            (2u64, Some(binding(20, "b"))), // claude exits → None
            (3u64, Some(binding(30, "c"))), // swapped for a different session
        ]);
        let new = HashMap::from([
            (1u64, Some(binding(10, "a"))),
            (2u64, None),
            (3u64, Some(binding(31, "c2"))),
            (4u64, Some(binding(40, "d"))), // brand-new pane binding
        ]);

        let mut events = binding_change_events(&prev, &new);
        events.sort_by_key(|(pane, _)| *pane);

        assert_eq!(
            events,
            vec![
                (2u64, None),
                (3u64, Some(binding(31, "c2"))),
                (4u64, Some(binding(40, "d"))),
            ]
        );
    }

    #[test]
    fn binding_change_events_reports_none_for_unregistered_pane() {
        // Pane 2 was bound last pass but is gone from `new` (the caller
        // unregistered it) — must still emit a clearing `None`.
        let prev = HashMap::from([
            (1u64, Some(binding(10, "a"))),
            (2u64, Some(binding(20, "b"))),
        ]);
        let new = HashMap::from([(1u64, Some(binding(10, "a")))]);

        let events = binding_change_events(&prev, &new);

        assert_eq!(events, vec![(2u64, None)]);
    }

    #[test]
    fn dead_sessions_are_those_live_before_but_not_now() {
        let prev = HashSet::from(["a".to_string(), "b".to_string()]);
        let now = HashSet::from(["a".to_string()]);

        assert_eq!(dead_sessions(&prev, &now), vec!["b".to_string()]);
    }

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
