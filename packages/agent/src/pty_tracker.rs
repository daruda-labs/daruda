//! Track which `claude` process lives inside each daruda pane, and when a
//! process once seen running a session stops running it.
//!
//! Session-file changes rebind panes; a known process gets a cheap check each
//! second, and a task launch rescans only while it waits to bind. Detaching a
//! pane or deleting a status file never proves exit.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::pty_link;

/// Pane id mirrored locally to keep this tracker independent of GPUI/workspace.
pub type PaneId = u64;

/// Coalescing window for register fan-out and atomic-rename wake bursts.
const DEBOUNCE: Duration = Duration::from_millis(100);
const PROCESS_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// How long a task pane keeps rescanning for a CLI that has not bound yet —
/// covers a slow first launch. Past it, a `claude` that never started stops
/// costing a rescan a second; hook and FSEvents pokes still resolve it.
const TASK_DISCOVERY_WINDOW: Duration = Duration::from_secs(120);

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
    /// A previously observed PID no longer runs its session: gone from the
    /// OS process table, or its session file is gone or names another session.
    SessionProcessExited { session_id: String, claude_pid: u32 },
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
/// exits when the last clone drops (via `ShutdownOnDrop`) or the
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
    panes: HashMap<PaneId, PaneEntry>,
    /// Last-known per-pane binding. Used to suppress duplicate
    /// `BindingChanged` events when nothing actually changed.
    bindings: HashMap<PaneId, Option<PtyBinding>>,
    /// Retained after pane detachment until the OS confirms exit.
    known_processes: HashMap<String, KnownProcess>,
}

/// One registered pane: its PTY shell, and the task launch it hosts if any.
#[derive(Clone, Debug, PartialEq)]
struct PaneEntry {
    shell_pid: u32,
    task: Option<TaskLaunch>,
}

/// A task CLI expected in a pane. Its account's session directory is scanned
/// for as long as the pane lives, since the FSEvents watch covers only the
/// default directory; the rescan-a-second discovery stops at `discover_until`
/// or at the first binding.
#[derive(Clone, Debug, PartialEq)]
struct TaskLaunch {
    sessions_dir: PathBuf,
    discover_until: Option<Instant>,
}

impl TrackerInner {
    fn discovering(&self) -> bool {
        self.panes.values().any(|pane| {
            pane.task
                .as_ref()
                .is_some_and(|t| t.discover_until.is_some())
        })
    }
}

/// A `claude` seen running a session, and the directory its session file
/// lives in — the file is how a reused PID is told apart from the original.
#[derive(Clone, Debug, PartialEq)]
struct KnownProcess {
    pid: u32,
    sessions_dir: PathBuf,
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
        lock_inner(&self.inner)
            .panes
            .entry(pane_id)
            .and_modify(|pane| pane.shell_pid = root_pid)
            .or_insert(PaneEntry {
                shell_pid: root_pid,
                task: None,
            });
        self.poke();
    }

    /// Unregister a pane (typically when the pane is closed) and wake
    /// the tracker so any binding it had is cleared with
    /// `BindingChanged { binding: None }`.
    pub fn unregister(&self, pane_id: PaneId) {
        lock_inner(&self.inner).panes.remove(&pane_id);
        self.poke();
    }

    /// Scan `sessions_dir` for the pane's task CLI and poll until it binds;
    /// an account-scoped directory has no FSEvents watch of its own. A pane
    /// not registered yet is ignored — its shell is what the walk ends at.
    pub fn track_task(&self, pane_id: PaneId, sessions_dir: PathBuf) {
        if let Some(pane) = lock_inner(&self.inner).panes.get_mut(&pane_id) {
            pane.task = Some(TaskLaunch {
                sessions_dir,
                discover_until: Some(Instant::now() + TASK_DISCOVERY_WINDOW),
            });
        }
        self.poke();
    }

    /// Restore a previously observed process without relying on a new PTY.
    pub fn track_session(&self, session_id: String, pid: u32, sessions_dir: PathBuf) {
        lock_inner(&self.inner)
            .known_processes
            .insert(session_id, KnownProcess { pid, sessions_dir });
        self.poke();
    }

    /// Whether a process observed running `session_id` has not been confirmed
    /// gone yet.
    pub fn is_running(&self, session_id: &str) -> bool {
        lock_inner(&self.inner)
            .known_processes
            .contains_key(session_id)
    }

    pub fn poke(&self) {
        // SILENT-OK: a dead channel means the tracker thread already
        // exited (Workspace teardown) — nothing left to wake.
        let _ = self.wake_tx.send(Wake::Poke);
    }

    /// Test-only introspection — the currently registered pane ids.
    #[cfg(any(test, feature = "test-support"))]
    pub fn tracked_pane_ids(&self) -> Vec<PaneId> {
        self.inner
            .lock()
            .map(|inner| inner.panes.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Test-only introspection — the PID awaiting OS exit for `session_id`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn known_process(&self, session_id: &str) -> Option<u32> {
        lock_inner(&self.inner)
            .known_processes
            .get(session_id)
            .map(|known| known.pid)
    }

    /// Test-only introspection — whether `pane_id` is still waiting for its
    /// task CLI to bind.
    #[cfg(any(test, feature = "test-support"))]
    pub fn awaits_binding(&self, pane_id: PaneId) -> bool {
        lock_inner(&self.inner)
            .panes
            .get(&pane_id)
            .and_then(|pane| pane.task.as_ref())
            .is_some_and(|task| task.discover_until.is_some())
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
        let (poll, discovering) = {
            let mut guard = lock_inner(&inner);
            expire_discovery(&mut guard.panes, Instant::now());
            let discovering = guard.discovering();
            (
                discovering || !guard.known_processes.is_empty(),
                discovering,
            )
        };
        let wake = if poll {
            wake_rx.recv_timeout(PROCESS_CHECK_INTERVAL)
        } else {
            wake_rx
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        };
        match wake {
            Ok(Wake::Poke) => {}
            // A quiet second only has to ask whether known processes still
            // run; the full rescan waits for a poke unless a task is binding.
            Err(mpsc::RecvTimeoutError::Timeout) if !discovering => {
                if !emit_exits(&inner, &event_tx) {
                    return;
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Ok(Wake::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
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
/// Intentionally NOT built on the app's `dir_watch::spawn_dir_watcher`: any
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
/// pass, and emit `BindingChanged`; then emit `SessionProcessExited` for
/// every known PID the OS no longer has. Returns `false`
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
    // empty, so any lingering bindings flush to `None` exactly once; known
    // processes outlive their pane and are still checked below.
    let (panes, task_dirs): (HashMap<PaneId, u32>, HashSet<PathBuf>) = {
        let guard = lock_inner(inner);
        (
            guard
                .panes
                .iter()
                .map(|(id, pane)| (*id, pane.shell_pid))
                .collect(),
            guard
                .panes
                .values()
                .filter_map(|pane| pane.task.as_ref())
                .map(|task| task.sessions_dir.clone())
                .filter(|dir| dir.as_path() != sessions_dir)
                .collect(),
        )
    };

    // Which directory each session file came from, so a bound process can
    // later be checked against its own file.
    let mut dir_of_pid: HashMap<u32, PathBuf> = HashMap::new();
    let new_bindings = if panes.is_empty() {
        HashMap::new()
    } else {
        let mut sessions = Vec::new();
        for dir in std::iter::once(sessions_dir.to_path_buf()).chain(task_dirs) {
            for meta in list_session_metas(&dir) {
                dir_of_pid.insert(meta.pid, dir.clone());
                sessions.push(meta);
            }
        }
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
    let PassOutcome {
        binding_events,
        exited,
    } = commit_pass(
        &mut lock_inner(inner),
        new_bindings,
        &|pid| {
            dir_of_pid
                .get(&pid)
                .cloned()
                .unwrap_or_else(|| sessions_dir.to_path_buf())
        },
        &still_runs,
    );

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
    send_exits(event_tx, exited)
}

/// The per-second check between rescans: confirm and report exits only.
fn emit_exits(inner: &Arc<Mutex<TrackerInner>>, event_tx: &mpsc::Sender<PtyTrackerEvent>) -> bool {
    let exited = confirmed_exits(&mut lock_inner(inner).known_processes, &still_runs);
    send_exits(event_tx, exited)
}

fn send_exits(event_tx: &mpsc::Sender<PtyTrackerEvent>, exited: Vec<(String, u32)>) -> bool {
    for (session_id, claude_pid) in exited {
        if event_tx
            .send(PtyTrackerEvent::SessionProcessExited {
                session_id,
                claude_pid,
            })
            .is_err()
        {
            return false;
        }
    }
    true
}

/// The OS-facing liveness rule: the PID exists and is still the process its
/// session file names.
fn still_runs(session_id: &str, known: &KnownProcess) -> bool {
    daruda_core::process::is_alive(known.pid)
        && pty_link::pid_holds_session(&known.sessions_dir, known.pid, session_id)
}

/// Stop the discovery rescan for task panes whose window has passed.
fn expire_discovery(panes: &mut HashMap<PaneId, PaneEntry>, now: Instant) {
    for task in panes.values_mut().filter_map(|pane| pane.task.as_mut()) {
        if task.discover_until.is_some_and(|deadline| now >= deadline) {
            task.discover_until = None;
        }
    }
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
/// resolves.
fn rescan(
    panes: &HashMap<PaneId, u32>,
    sessions: &[pty_link::PidSessionMeta],
    parent_of: &dyn Fn(u32) -> Option<u32>,
) -> HashMap<PaneId, Option<PtyBinding>> {
    let resolved = resolve_pane_bindings(panes, sessions, parent_of);
    panes
        .keys()
        .map(|pane_id| (*pane_id, resolved.get(pane_id).cloned()))
        .collect()
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

/// What one resolution pass has to report.
struct PassOutcome {
    binding_events: Vec<(PaneId, Option<PtyBinding>)>,
    /// `(session_id, claude_pid)` for every known process no longer running.
    exited: Vec<(String, u32)>,
}

/// Fold one pass's bindings into the tracker state: every bound PID becomes
/// known until the OS confirms it gone, and a bound task pane stops the
/// discovery poll.
fn commit_pass(
    inner: &mut TrackerInner,
    new_bindings: HashMap<PaneId, Option<PtyBinding>>,
    dir_of_pid: &dyn Fn(u32) -> PathBuf,
    still_runs: &dyn Fn(&str, &KnownProcess) -> bool,
) -> PassOutcome {
    let binding_events = binding_change_events(&inner.bindings, &new_bindings);
    for (pane_id, binding) in &new_bindings {
        if let Some(binding) = binding {
            if let Some(task) = inner.panes.get_mut(pane_id).and_then(|p| p.task.as_mut()) {
                task.discover_until = None;
            }
            inner.known_processes.insert(
                binding.session_id.clone(),
                KnownProcess {
                    pid: binding.claude_pid,
                    sessions_dir: dir_of_pid(binding.claude_pid),
                },
            );
        }
    }
    inner.bindings = new_bindings;
    let exited = confirmed_exits(&mut inner.known_processes, still_runs);
    PassOutcome {
        binding_events,
        exited,
    }
}

/// Forget only processes confirmed gone, independent of pane bindings.
fn confirmed_exits(
    known: &mut HashMap<String, KnownProcess>,
    still_runs: &dyn Fn(&str, &KnownProcess) -> bool,
) -> Vec<(String, u32)> {
    let exited: Vec<_> = known
        .iter()
        .filter(|(session, process)| !still_runs(session, process))
        .map(|(session, process)| (session.clone(), process.pid))
        .collect();
    for (session, _) in &exited {
        known.remove(session);
    }
    exited
}

#[cfg(test)]
#[path = "pty_tracker_tests.rs"]
mod tests;
