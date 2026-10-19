//! The wait between a dead run's last record and its resume, so the agent
//! processes it left exit before the run starts beside them.

use std::path::Path;
use std::time::Duration;

use crate::run_log::reader;

/// How long after a dead run's last record a resume waits before it starts.
/// Measured: after the host is killed, an agent's CLI lingers as an orphan
/// for 3–10 s before it exits, and a resume should not start beside it.
pub(crate) const SETTLE_AFTER_CRASH: Duration = Duration::from_secs(10);

/// What is left of `settle` after a record written at `last_ms`, at `now_ms`.
/// A record dated after now (a clock set back) waits the whole window, never
/// longer.
pub(crate) fn settle_wait(last_ms: u64, now_ms: u64, settle: Duration) -> Duration {
    let since = Duration::from_millis(now_ms.saturating_sub(last_ms));
    settle.saturating_sub(since)
}

/// Whether a resume may go on once the settle window is behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settled {
    Go,
    /// Stopped before the run was taken. Nothing was locked or written, so
    /// the run is left resumable exactly as it was.
    Stopped,
}

/// Wait out what is left of the settle window after a dead run's last
/// record, so its agent processes exit before the run starts beside them.
/// Only for a run that stopped without ending; a stop cuts the wait short.
///
/// Before the lock, and reading only when the log was last written: a
/// stop during the wait must leave no trace, not even a reclaimed lock.
pub(crate) fn settle_after_crash(
    run_dir: &Path,
    settle: Duration,
    events: Option<&smol::channel::Sender<crate::event::RunNotice>>,
    cancel: &crate::runner::CancelToken,
) -> Settled {
    if let Some(last_ms) = reader::last_written_while_running(run_dir) {
        wait_out(last_ms, settle, events, cancel);
    }
    // Also with no wait at all: a stop that lands before the run is taken
    // must not end, in its log, a run that never ran again.
    if cancel.is_canceled() {
        Settled::Stopped
    } else {
        Settled::Go
    }
}

fn wait_out(
    last_ms: u64,
    settle: Duration,
    events: Option<&smol::channel::Sender<crate::event::RunNotice>>,
    cancel: &crate::runner::CancelToken,
) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        });
    let wait = settle_wait(last_ms, now_ms, settle);
    if wait.is_zero() {
        return;
    }
    crate::event::emit(events, crate::event::RunNotice::Settling { wait });
    let until = std::time::Instant::now() + wait;
    while !cancel.is_canceled() {
        let left = until.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break;
        }
        std::thread::sleep(left.min(Duration::from_millis(100)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wait is what is left of the window after the dead run's last
    /// record; a clock set back never stretches it past the window.
    #[test]
    fn the_settle_wait_is_what_is_left_of_the_window() {
        let window = Duration::from_secs(10);
        assert_eq!(settle_wait(1_000, 4_000, window), Duration::from_secs(7));
        assert_eq!(settle_wait(1_000, 11_000, window), Duration::ZERO);
        assert_eq!(settle_wait(1_000, 90_000, window), Duration::ZERO);
        assert_eq!(settle_wait(9_000, 1_000, window), window);
        assert_eq!(settle_wait(1_000, 4_000, Duration::ZERO), Duration::ZERO);
    }
}
