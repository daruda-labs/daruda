//! Confirm exits of agent processes already named by hooks or durable runs.
//! No pane discovery, process-tree walks, directory scans, or file watches.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::pty_link;

const PROCESS_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// A pane's hook-reported session and its observed PID, when available.
#[derive(Clone, Debug, PartialEq)]
pub struct PtyBinding {
    pub terminal_id: Option<crate::hooks::route::TerminalId>,
    pub claude_pid: Option<u32>,
    pub session_id: String,
}

impl PtyBinding {
    pub fn storage_key(&self) -> &str {
        self.terminal_id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or(&self.session_id)
    }
}

/// An observed process no longer runs the session it was registered for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessExited {
    pub session_id: String,
    pub pid: u32,
}

/// Known processes outlive pane/window detachment. Dropping the last handle
/// disconnects the wake channel and stops the worker, even while it is idle.
#[derive(Clone)]
pub struct PtyTracker {
    inner: Arc<Mutex<HashMap<(String, u32), KnownProcess>>>,
    wake_tx: mpsc::Sender<()>,
}

#[derive(Clone, Debug, PartialEq)]
struct KnownProcess {
    pid: u32,
    proof: ProcessProof,
}

#[derive(Clone, Debug, PartialEq)]
enum ProcessProof {
    /// Restored Claude runs have a PID and account but no live terminal.
    SessionFile(PathBuf),
    Observed {
        started_at: Option<u64>,
    },
}

impl PtyTracker {
    pub fn spawn() -> (Self, mpsc::Receiver<ProcessExited>) {
        let (event_tx, event_rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::channel();
        let inner = Arc::new(Mutex::new(HashMap::new()));
        let worker = inner.clone();
        std::thread::spawn(move || {
            loop {
                let idle = lock_inner(&worker).is_empty();
                let wake = if idle {
                    wake_rx
                        .recv()
                        .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                } else {
                    wake_rx.recv_timeout(PROCESS_CHECK_INTERVAL)
                };
                if matches!(wake, Err(mpsc::RecvTimeoutError::Disconnected)) {
                    break;
                }
                while wake_rx.try_recv().is_ok() {}
                let exited = confirmed_exits(&mut lock_inner(&worker), &still_runs);
                for event in exited {
                    if event_tx.send(event).is_err() {
                        return;
                    }
                }
            }
        });
        (Self { inner, wake_tx }, event_rx)
    }

    /// Restore an exact PID claim, checking only its account's single session
    /// file. This never discovers a different PID or attaches a pane.
    pub fn track_session(&self, session_id: String, pid: u32, sessions_dir: PathBuf) {
        lock_inner(&self.inner).insert(
            (session_id, pid),
            KnownProcess {
                pid,
                proof: ProcessProof::SessionFile(sessions_dir),
            },
        );
        self.poke();
    }

    /// Observe the PID named by a routed hook; birth time guards PID reuse.
    pub fn track_agent(&self, session_id: String, pid: u32) {
        let started_at = process_started_at(pid);
        lock_inner(&self.inner).insert(
            (session_id, pid),
            KnownProcess {
                pid,
                proof: ProcessProof::Observed { started_at },
            },
        );
        self.poke();
    }

    pub fn is_running(&self, session_id: &str) -> bool {
        lock_inner(&self.inner)
            .keys()
            .any(|(session, _)| session == session_id)
    }

    pub fn poke(&self) {
        // SILENT-OK: the receiver can have gone away with its consumer.
        let _ = self.wake_tx.send(());
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn known_process(&self, session_id: &str) -> Option<u32> {
        lock_inner(&self.inner)
            .iter()
            .find(|((session, _), _)| session == session_id)
            .map(|(_, known)| known.pid)
    }
}

fn confirmed_exits(
    known: &mut HashMap<(String, u32), KnownProcess>,
    still_runs: &dyn Fn(&str, &KnownProcess) -> bool,
) -> Vec<ProcessExited> {
    let exited: Vec<_> = known
        .iter()
        .filter(|(session, process)| !still_runs(&session.0, process))
        .map(|(session, process)| ProcessExited {
            session_id: session.0.clone(),
            pid: process.pid,
        })
        .collect();
    for event in &exited {
        known.remove(&(event.session_id.clone(), event.pid));
    }
    exited
}

fn still_runs(session_id: &str, known: &KnownProcess) -> bool {
    if !daruda_core::process::is_alive(known.pid) {
        return false;
    }
    match &known.proof {
        ProcessProof::SessionFile(dir) => pty_link::pid_holds_session(dir, known.pid, session_id),
        ProcessProof::Observed { started_at } => {
            // A denied process query is not proof of exit. Compare birth
            // times whenever both observations are available.
            match (started_at, process_started_at(known.pid)) {
                (Some(expected), Some(actual)) => *expected == actual,
                _ => true,
            }
        }
    }
}

fn process_started_at(pid: u32) -> Option<u64> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut system = System::new().ok()?;
    let pid = Pid::from_u32(pid);
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    system.process(pid).map(|process| process.start_time())
}

fn lock_inner(
    inner: &Mutex<HashMap<(String, u32), KnownProcess>>,
) -> std::sync::MutexGuard<'_, HashMap<(String, u32), KnownProcess>> {
    inner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
#[path = "pty_tracker_tests.rs"]
mod tests;
