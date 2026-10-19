//! How long a bridge waits after a failed fetch or command claim before it
//! tries the same thing again.
//!
//! Short first, because the common failure is a single dropped long poll or a
//! transient storage error, and a message the phone already sent waits out
//! the whole delay. Doubling after that, because the other failures — a
//! revoked token, a full disk — are not helped by asking again at once.

use std::time::Duration;

/// Wait after the first failure.
const FIRST: Duration = Duration::from_secs(1);

/// Ceiling, matching the bridges' idle and reconnect cadence so a bot that
/// stays broken costs no more than a bot that is switched off.
const CEILING: Duration = Duration::from_secs(30);

/// Grows while fetches keep failing, back to [`FIRST`] on the next success.
pub(crate) struct ErrorBackoff {
    next: Duration,
}

impl ErrorBackoff {
    pub(crate) const fn new() -> Self {
        Self { next: FIRST }
    }

    /// The wait for the failure just seen, and arm the next one.
    pub(crate) fn delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (delay * 2).min(CEILING);
        delay
    }

    pub(crate) fn reset(&mut self) {
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
