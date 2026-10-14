//! Whether the computer should stay awake, from the agent sessions every
//! window reports. GPUI-free: values in, a decision out.
//!
//! Each session counts only for a bounded time in one status, so a session
//! stuck in `Working` or an approval nobody answers cannot hold the machine
//! awake forever.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use daruda_agent::SessionStatus;
use daruda_config::KeepAwakeMode;
use daruda_store::project::WorkspaceUuid;

/// How long one uninterrupted `Working` / `ExecutingTool` status counts.
/// Terminal Claude flips between the two per tool call, renewing it.
const WORKING_CAP: Duration = Duration::from_secs(2 * 60 * 60);

/// A session as seen across windows. A chat pane's `acp:{pane_id}` is unique
/// only within its window, hence the workspace.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SessionKey {
    pub(crate) workspace: WorkspaceUuid,
    pub(crate) session: String,
}

/// One session's status this round.
#[derive(Clone, Debug)]
pub(crate) struct Observation {
    pub(crate) key: SessionKey,
    pub(crate) status: SessionStatus,
}

/// Each tracked session's status and when it entered that status.
#[derive(Default)]
pub(crate) struct SessionClocks(HashMap<SessionKey, (SessionStatus, Instant)>);

/// How long each counting status keeps the computer awake.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Caps {
    pub(crate) working: Duration,
    pub(crate) approval_wait: Duration,
}

impl Caps {
    pub(crate) fn from_config(desktop: &daruda_config::desktop::DesktopConfig) -> Self {
        Self {
            working: WORKING_CAP,
            approval_wait: Duration::from_secs(
                desktop.keep_awake_approval_wait_minutes.saturating_mul(60),
            ),
        }
    }
}

/// The outcome, with the sessions currently counting toward it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Decision {
    pub(crate) active: bool,
    pub(crate) working: usize,
    pub(crate) waiting: usize,
}

/// Fold this round's observations into `clocks` and decide. `Off` forgets
/// every session; `On` stays active but still counts sessions for display.
pub(crate) fn decide(
    mode: KeepAwakeMode,
    observations: impl IntoIterator<Item = Observation>,
    clocks: &mut SessionClocks,
    caps: Caps,
    now: Instant,
) -> Decision {
    if mode == KeepAwakeMode::Off {
        clocks.0.clear();
        return Decision::default();
    }
    let mut next = HashMap::new();
    for Observation { key, status } in observations {
        let since = match clocks.0.get(&key) {
            Some(&(previous, since)) if previous == status => since,
            _ => now,
        };
        next.insert(key, (status, since));
    }
    clocks.0 = next;

    let mut decision = Decision::default();
    for &(status, since) in clocks.0.values() {
        let age = now.saturating_duration_since(since);
        match status {
            SessionStatus::Working | SessionStatus::ExecutingTool if age < caps.working => {
                decision.working += 1;
            }
            SessionStatus::NeedsAttention if age < caps.approval_wait => decision.waiting += 1,
            _ => {}
        }
    }
    decision.active = mode == KeepAwakeMode::On || decision.working + decision.waiting > 0;
    decision
}

#[cfg(test)]
mod tests;
