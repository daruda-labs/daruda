//! Schedule only transcript snapshots whose status can change while quiet.

use super::*;
use std::collections::HashMap;
use std::time::Duration;

struct Pending {
    observation: Observation,
    deadline: DateTime<Utc>,
}

fn publish(
    mut observation: Observation,
    permissions: &PermissionChecker,
    events: &mpsc::Sender<JsonlEvent>,
    pending: &mut HashMap<PathBuf, Pending>,
    now: DateTime<Utc>,
) -> bool {
    let classification = classify_status_at(&observation.entries, permissions, now);
    observation.event.status = classification.status;
    // Only the status ages: retain the original transcript timestamp so a
    // later hook remains authoritative when this delayed event reaches it.
    if events.send(observation.event.clone()).is_err() {
        return false;
    }
    let path = observation.event.jsonl_path.clone();
    pending.remove(&path);
    if let Some(deadline) = classification.next_transition_at {
        pending.insert(
            path,
            Pending {
                observation,
                deadline,
            },
        );
    }
    true
}

/// The receive boundary admits a clock-controlled driver without real sleeps.
pub(super) fn forward(
    initial: Vec<Observation>,
    permissions: &PermissionChecker,
    events: mpsc::Sender<JsonlEvent>,
    now: impl Fn() -> DateTime<Utc>,
    mut receive: impl FnMut(Option<Duration>) -> Result<JsonlRaw, mpsc::RecvTimeoutError>,
    mut rescan: impl FnMut(DateTime<Utc>) -> Vec<Observation>,
) {
    let mut pending: HashMap<PathBuf, Pending> = HashMap::new();
    for observation in initial {
        if !publish(observation, permissions, &events, &mut pending, now()) {
            return;
        }
    }
    loop {
        let timeout = pending
            .values()
            .map(|record| record.deadline)
            .min()
            .map(|deadline| (deadline - now()).to_std().unwrap_or_default());
        // recv_timeout delivers buffered file events before reporting Timeout,
        // even at zero duration. A newer snapshot replaces its pending record
        // before any old cached deadline can be published.
        match receive(timeout) {
            Ok(JsonlRaw::Event(observation)) => {
                if !publish(observation, permissions, &events, &mut pending, now()) {
                    return;
                }
            }
            Ok(JsonlRaw::Forget(path)) => {
                pending.remove(&path);
            }
            Ok(JsonlRaw::Rescan) => {
                pending.clear();
                for observation in rescan(now()) {
                    if !publish(observation, permissions, &events, &mut pending, now()) {
                        return;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let now = now();
                let due: Vec<_> = pending
                    .iter()
                    .filter(|(_, record)| record.deadline <= now)
                    .map(|(path, _)| path.clone())
                    .collect();
                for path in due {
                    if let Some(record) = pending.remove(&path)
                        && !publish(record.observation, permissions, &events, &mut pending, now)
                    {
                        return;
                    }
                }
            }
            // Dropping DirWatcher disconnects this channel, cancelling timers
            // immediately rather than keeping the old Workspace alive.
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests;
