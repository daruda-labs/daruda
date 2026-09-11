//! How long daruda has been out of the foreground.
//!
//! [`super::attention::is_app_active`] answers "is daruda foreground *right
//! now*" — one sample. A gate that must not fire on a blink needs the other
//! half: how long that answer has held. This file is that half and nothing
//! else, so it stays pure and testable without a live `NSApplication`.

use std::time::{Duration, Instant};

/// App presence, with absence carrying its own start time.
///
/// A bare `bool` reads `false` both when the user left and when the window
/// blinked. Pairing it with an `Option<Instant>` meaningful only while
/// absent would leave the invalid combination representable, so the
/// timestamp lives inside the absent variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// daruda is the foreground app.
    Here,
    /// daruda has not been foreground since `since`.
    Away { since: Instant },
}

impl Presence {
    /// Fold one observation in. `since` is stamped only on the
    /// `Here -> Away` edge — an absence that keeps being observed must not
    /// restart its own clock, or a grace window never fills.
    pub fn observe(self, app_active: bool, now: Instant) -> Self {
        match (self, app_active) {
            (_, true) => Self::Here,
            (Self::Here, false) => Self::Away { since: now },
            (away @ Self::Away { .. }, false) => away,
        }
    }

    /// A foreground sample never counts as absence, even with zero grace.
    pub fn away_for_at_least(self, grace: Duration, now: Instant) -> bool {
        match self {
            Self::Here => false,
            Self::Away { since } => now.saturating_duration_since(since) >= grace,
        }
    }

    /// Seconds of unbroken absence, or `None` while present.
    pub fn away_secs(self, now: Instant) -> Option<f64> {
        match self {
            Self::Here => None,
            Self::Away { since } => Some(now.saturating_duration_since(since).as_secs_f64()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaving_the_foreground_stamps_when_absence_started() {
        let t0 = Instant::now();
        let away = Presence::Here.observe(false, t0);
        assert_eq!(away, Presence::Away { since: t0 });
        assert_eq!(away.away_secs(t0), Some(0.0));
        assert_eq!(away.away_secs(t0 + Duration::from_secs(15)), Some(15.0));
    }

    #[test]
    fn a_continuing_absence_never_restarts_its_own_clock() {
        // The whole point: the flush pump re-observes every 15s. If each
        // observation re-stamped `since`, absence would never accumulate.
        let t0 = Instant::now();
        let mut p = Presence::Here.observe(false, t0);
        for step in 1..=4 {
            p = p.observe(false, t0 + Duration::from_secs(step * 5));
        }
        assert_eq!(p, Presence::Away { since: t0 });
        assert_eq!(p.away_secs(t0 + Duration::from_secs(20)), Some(20.0));
    }

    #[test]
    fn returning_resets_absence_so_the_next_one_is_measured_fresh() {
        let t0 = Instant::now();
        let blinked = Presence::Here
            .observe(false, t0)
            .observe(true, t0 + Duration::from_secs(5));
        assert_eq!(blinked, Presence::Here);

        let again = blinked.observe(false, t0 + Duration::from_secs(10));
        // The earlier 5s must not count toward this absence.
        assert_eq!(again.away_secs(t0 + Duration::from_secs(20)), Some(10.0));
    }

    #[test]
    fn being_foreground_is_not_an_absence_of_zero_length() {
        let t0 = Instant::now();
        assert_eq!(Presence::Here.observe(true, t0), Presence::Here);
        assert_eq!(Presence::Here.away_secs(t0), None);
    }

    #[test]
    fn sustained_absence_requires_the_full_grace_and_resets_on_return() {
        let t0 = Instant::now();
        let grace = Duration::from_secs(15);
        let away = Presence::Here.observe(false, t0);
        assert!(!away.away_for_at_least(grace, t0 - Duration::from_secs(1)));
        assert!(!away.away_for_at_least(grace, t0 + grace - Duration::from_nanos(1)));
        assert!(away.away_for_at_least(grace, t0 + grace));
        assert!(away.away_for_at_least(Duration::ZERO, t0));

        let returned = away.observe(true, t0 + Duration::from_secs(5));
        assert!(!returned.away_for_at_least(grace, t0 + grace));
        assert!(!returned.away_for_at_least(Duration::ZERO, t0 + grace));
    }
}
