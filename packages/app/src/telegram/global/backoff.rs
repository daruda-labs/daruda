//! How long the poll loop waits after a failed `getUpdates`.
//!
//! Short first, because the common failure is a single dropped long poll on a
//! network that is otherwise fine, and a message the phone already sent waits
//! out the whole delay. Doubling after that, because the other failures — a
//! revoked token, Telegram down — are not helped by asking again at once.

use std::time::Duration;

/// Wait after the first failure.
const FIRST: Duration = Duration::from_secs(1);

/// Ceiling, matching the bridge's idle cadence so a bot that stays broken costs
/// no more than a bot that is switched off.
const CEILING: Duration = super::IDLE_RECHECK;

/// Grows while fetches keep failing, back to [`FIRST`] on the next success.
pub(super) struct ErrorBackoff {
    next: Duration,
}

impl ErrorBackoff {
    pub(super) const fn new() -> Self {
        Self { next: FIRST }
    }

    /// The wait for the failure just seen, and arm the next one.
    pub(super) fn delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (delay * 2).min(CEILING);
        delay
    }

    pub(super) fn reset(&mut self) {
        self.next = FIRST;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_failure_waits_the_least_and_later_ones_double_to_the_ceiling() {
        let mut backoff = ErrorBackoff::new();
        let mut seen = Vec::new();
        for _ in 0..8 {
            seen.push(backoff.delay().as_secs());
        }
        assert_eq!(seen, vec![1, 2, 4, 8, 16, 30, 30, 30]);
    }

    #[test]
    fn a_success_starts_the_ladder_over() {
        let mut backoff = ErrorBackoff::new();
        backoff.delay();
        backoff.delay();
        backoff.reset();
        assert_eq!(backoff.delay(), FIRST);
    }
}
