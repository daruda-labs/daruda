//! What "the user is away from daruda" means, and nothing else.
//!
//! [`super::attention::is_app_active`] is one sample; the private `Presence`
//! is how long that sample has held, so a gate need not fire on a blink.
//! Foreground state alone still cannot tell "left the machine" from "working
//! in another app", so [`AwaySignal`] folds in input idleness and answers the
//! one question callers have — [`AwaySignal::is_away`]. `Presence` stays
//! module-private so half the evidence is not reachable on its own.
//!
//! Pure and testable without a live `NSApplication` or HID query.

use std::time::{Duration, Instant};

/// App presence, with absence carrying its own start time.
///
/// A bare `bool` reads `false` both when the user left and when the window
/// blinked. Pairing it with an `Option<Instant>` meaningful only while
/// absent would leave the invalid combination representable, so the
/// timestamp lives inside the absent variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presence {
    /// daruda is the foreground app.
    Here,
    /// daruda has not been foreground since `since`.
    Away { since: Instant },
}

impl Presence {
    /// Fold one observation in. `since` is stamped only on the
    /// `Here -> Away` edge — an absence that keeps being observed must not
    /// restart its own clock, or a grace window never fills.
    fn observe(self, app_active: bool, now: Instant) -> Self {
        match (self, app_active) {
            (_, true) => Self::Here,
            (Self::Here, false) => Self::Away { since: now },
            (away @ Self::Away { .. }, false) => away,
        }
    }

    /// A foreground sample never counts as absence, even with zero grace.
    fn away_for_at_least(self, grace: Duration, now: Instant) -> bool {
        match self {
            Self::Here => false,
            Self::Away { since } => now.saturating_duration_since(since) >= grace,
        }
    }

    /// Seconds of unbroken absence, or `None` while present.
    fn away_secs(self, now: Instant) -> Option<f64> {
        match self {
            Self::Here => None,
            Self::Away { since } => Some(now.saturating_duration_since(since).as_secs_f64()),
        }
    }
}

/// What absence means. Idleness decides; the foreground signal only chooses
/// *which* idle bar applies. See `daruda_config::PresenceConfig` for why
/// requiring the blur outright is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AwayRule {
    /// How long daruda must stay out of the foreground before the lower bar
    /// applies. Absorbs a focus blink, including one no human caused —
    /// another process taking the foreground raises the same edge a
    /// deliberate app switch does.
    pub grace: Duration,
    /// Idle bar once that blur is sustained: the user is demonstrably in some
    /// other window, so silence needs less corroboration.
    pub idle_bar: Duration,
    /// Idle bar while daruda is still frontmost, where silence is as likely
    /// to be reading as leaving. Never below [`Self::idle_bar`] — see
    /// `PresenceConfig::clamp`.
    pub foreground_idle_bar: Duration,
}

/// Both absence signals, folded. Holding them in one value is what keeps
/// [`Self::is_away`] a question with no arguments beyond the rule and the
/// clock — a caller cannot accidentally answer it from half the evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AwaySignal {
    blur: Presence,
    /// `None` when the platform cannot report input idleness (see
    /// [`super::attention::system_idle_seconds`]).
    idle: Option<Duration>,
}

impl AwaySignal {
    /// Foreground, with no idleness accrued.
    pub const HERE: Self = Self {
        blur: Presence::Here,
        idle: Some(Duration::ZERO),
    };

    /// Fold one observation of both signals in. Idleness is a level the OS
    /// reports outright, so it replaces rather than accumulates; only the
    /// blur half carries state across observations.
    pub fn observe(self, app_active: bool, idle: Option<Duration>, now: Instant) -> Self {
        Self {
            blur: self.blur.observe(app_active, now),
            idle,
        }
    }

    /// Whether the user is away right now: input has been silent for at least
    /// the bar that the foreground state selects.
    ///
    /// A host that cannot report idleness falls back to the blur alone rather
    /// than failing the idle test forever, which would silence every
    /// presence-gated channel on that machine.
    pub fn is_away(self, rule: AwayRule, now: Instant) -> bool {
        let blurred = self.blur.away_for_at_least(rule.grace, now);
        let Some(idle) = self.idle else {
            return blurred;
        };
        idle >= if blurred {
            rule.idle_bar
        } else {
            rule.foreground_idle_bar
        }
    }

    /// Seconds of unbroken absence from the foreground, or `None` while
    /// present. For tracing — it is one half of the verdict, not the verdict.
    pub fn away_secs(self, now: Instant) -> Option<f64> {
        self.blur.away_secs(now)
    }

    /// The last observed input idleness, or `None` if the host cannot report
    /// it. For tracing.
    pub fn idle(self) -> Option<Duration> {
        self.idle
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

    fn rule(grace_secs: u64, idle_secs: u64, foreground_idle_secs: u64) -> AwayRule {
        AwayRule {
            grace: Duration::from_secs(grace_secs),
            idle_bar: Duration::from_secs(idle_secs),
            foreground_idle_bar: Duration::from_secs(foreground_idle_secs),
        }
    }

    /// The five states the two signals can describe, at the shipped defaults.
    /// Table-driven because the interesting property is the whole mapping,
    /// not any single row — and because row 4 is the one a blur-required rule
    /// silently loses.
    #[test]
    fn the_five_presence_states_map_to_the_intended_verdicts() {
        let t0 = Instant::now();
        let r = rule(10, 30, 180);
        let now = t0 + Duration::from_secs(600);
        let cases = [
            ("working in daruda", true, 0, false),
            ("reading in daruda", true, 60, false),
            // The logged incident: blurred past the grace, machine still in use.
            ("working in another app", false, 16, false),
            // The case a blur-required rule cannot reach.
            ("walked away, daruda frontmost", true, 180, true),
            ("walked away, another app frontmost", false, 180, true),
        ];
        for (name, app_active, idle_secs, want) in cases {
            let signal =
                AwaySignal::HERE.observe(app_active, Some(Duration::from_secs(idle_secs)), t0);
            assert_eq!(signal.is_away(r, now), want, "{name}");
        }
    }

    #[test]
    fn a_sustained_blur_lowers_the_bar_rather_than_deciding_on_its_own() {
        let t0 = Instant::now();
        let r = rule(15, 60, 300);
        let now = t0 + Duration::from_secs(600);

        // Blurred: the lower bar applies.
        let blurred = AwaySignal::HERE.observe(false, Some(Duration::from_secs(60)), t0);
        assert!(blurred.is_away(r, now));
        let blurred_busy = AwaySignal::HERE.observe(false, Some(Duration::from_secs(59)), t0);
        assert!(!blurred_busy.is_away(r, now));

        // Frontmost: the same 60s of silence is not enough on its own.
        let frontmost = AwaySignal::HERE.observe(true, Some(Duration::from_secs(60)), t0);
        assert!(!frontmost.is_away(r, now));
        let long_gone = AwaySignal::HERE.observe(true, Some(Duration::from_secs(300)), t0);
        assert!(long_gone.is_away(r, now));
    }

    #[test]
    fn a_blur_under_the_grace_still_uses_the_foreground_bar() {
        let t0 = Instant::now();
        let r = rule(15, 60, 300);
        // Blurred 14s — not yet sustained, so a blink cannot drop the bar.
        let blinked = AwaySignal::HERE.observe(false, Some(Duration::from_secs(60)), t0);
        assert!(!blinked.is_away(r, t0 + Duration::from_secs(14)));
        assert!(blinked.is_away(r, t0 + Duration::from_secs(15)));
    }

    #[test]
    fn a_zero_idle_bar_leaves_the_foreground_signal_alone_in_charge() {
        let t0 = Instant::now();
        let r = rule(15, 0, 0);
        let s = AwaySignal::HERE.observe(false, Some(Duration::ZERO), t0);
        assert!(s.is_away(r, t0 + Duration::from_secs(15)));
        // With both bars at zero the foreground case is away too — that is
        // what asking for no idle requirement means.
        let frontmost = AwaySignal::HERE.observe(true, Some(Duration::ZERO), t0);
        assert!(frontmost.is_away(r, t0));
    }

    #[test]
    fn returning_to_the_foreground_raises_the_bar_again() {
        let t0 = Instant::now();
        let r = rule(15, 60, 300);
        let gone = AwaySignal::HERE.observe(false, Some(Duration::from_secs(120)), t0);
        assert!(gone.is_away(r, t0 + Duration::from_secs(60)));

        // Focus comes back with the HID clock untouched: the stricter bar now
        // applies, and 120s does not clear it.
        let back = gone.observe(
            true,
            Some(Duration::from_secs(120)),
            t0 + Duration::from_secs(60),
        );
        assert!(!back.is_away(r, t0 + Duration::from_secs(120)));
    }

    #[test]
    fn idleness_is_a_level_that_replaces_rather_than_accumulating() {
        let t0 = Instant::now();
        let s = AwaySignal::HERE
            .observe(false, Some(Duration::from_secs(90)), t0)
            .observe(
                false,
                Some(Duration::from_secs(2)),
                t0 + Duration::from_secs(1),
            );
        assert_eq!(s.idle(), Some(Duration::from_secs(2)));
        // The blur half, by contrast, keeps its original start.
        assert_eq!(s.away_secs(t0 + Duration::from_secs(1)), Some(1.0));
    }

    #[test]
    fn an_unreportable_idle_reading_falls_back_to_the_blur_alone() {
        let t0 = Instant::now();
        let r = rule(15, 60, 300);
        // A host that cannot answer the idle query must not be read as a user
        // sitting at the keyboard, or the gate never opens on that machine.
        let blind = AwaySignal::HERE.observe(false, None, t0);
        assert!(!blind.is_away(r, t0 + Duration::from_secs(14)));
        assert!(blind.is_away(r, t0 + Duration::from_secs(15)));
        assert_eq!(blind.idle(), None);

        // Falling back is not forcing absence: a frontmost daruda still says no.
        let blind_here = AwaySignal::HERE.observe(true, None, t0);
        assert!(!blind_here.is_away(r, t0 + Duration::from_secs(600)));
    }
}
