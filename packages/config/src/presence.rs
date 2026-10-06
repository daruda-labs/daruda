use serde::{Deserialize, Serialize};

/// What "the user is away from daruda" means. Its own section rather than a
/// corner of `[telegram]`: a channel decides whether it cares about absence
/// (`telegram.only_when_away`); this decides what absence *is*, once, for
/// every channel that asks.
///
/// Two signals are available — whether daruda is frontmost, and how long
/// system input has been silent — and they cannot, on their own, tell
/// "reading a long answer in daruda" from "got up and left with daruda
/// frontmost". Both look like *foreground + idle*. So idleness decides, and
/// **daruda being frontmost only raises the bar** rather than settling the
/// question:
///
/// | daruda frontmost | bar that applies |
/// |---|---|
/// | no, for at least `away_grace_secs` | `away_idle_secs` |
/// | yes (or blurred under the grace) | `away_idle_foreground_secs` |
///
/// Requiring the blur outright instead would make "walked away from a
/// frontmost daruda" unreachable, which is the case the phone exists for.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct PresenceConfig {
    /// Continuous time out of the foreground before the lower bar applies.
    /// Absorbs a focus blink, including one no human caused — another process
    /// taking the foreground raises the same edge a deliberate switch does.
    /// Zero makes leaving the foreground count immediately.
    pub away_grace_secs: u64,
    /// Input idleness required once daruda has been out of the foreground for
    /// `away_grace_secs`. The user is demonstrably somewhere else, so this is
    /// the lower of the two bars.
    pub away_idle_secs: u64,
    /// Input idleness required while daruda is still frontmost, where silence
    /// is as likely to be reading as leaving. Higher than [`Self::away_idle_secs`]
    /// for that reason, and [`Self::clamp`] refuses to let it fall below —
    /// a blur must never make absence *harder* to reach.
    pub away_idle_foreground_secs: u64,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            away_grace_secs: 10,
            away_idle_secs: 30,
            away_idle_foreground_secs: 180,
        }
    }
}

impl PresenceConfig {
    /// Hold the one ordering invariant between the two bars. Without it a
    /// config could make blurring *reduce* the chance of being judged away,
    /// which inverts what the blur signal means.
    pub fn clamp(&mut self) {
        self.away_idle_foreground_secs = self.away_idle_foreground_secs.max(self.away_idle_secs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_put_the_foreground_bar_above_the_blurred_one() {
        let cfg = PresenceConfig::default();
        assert_eq!(cfg.away_grace_secs, 10);
        assert_eq!(cfg.away_idle_secs, 30);
        assert_eq!(cfg.away_idle_foreground_secs, 180);
        assert!(cfg.away_idle_foreground_secs > cfg.away_idle_secs);
    }

    #[test]
    fn clamp_refuses_a_foreground_bar_below_the_blurred_one() {
        let mut cfg = PresenceConfig {
            away_grace_secs: 15,
            away_idle_secs: 120,
            away_idle_foreground_secs: 30,
        };
        cfg.clamp();
        assert_eq!(cfg.away_idle_foreground_secs, 120);

        // A foreground bar already above the blurred one is left alone.
        let mut ordered = PresenceConfig::default();
        ordered.clamp();
        assert_eq!(ordered.away_idle_foreground_secs, 180);
    }

    #[test]
    fn toml_round_trip_preserves_explicit_zero() {
        let cfg: PresenceConfig = toml::from_str("away_idle_secs = 0\n").unwrap();
        assert_eq!(cfg.away_idle_secs, 0);
        // Unspecified fields fall back to their defaults via `#[serde(default)]`.
        assert_eq!(cfg.away_grace_secs, 10);
        assert_eq!(cfg.away_idle_foreground_secs, 180);

        let reparsed: PresenceConfig = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(reparsed.away_idle_secs, 0);
        assert_eq!(reparsed.away_grace_secs, 10);
        assert_eq!(reparsed.away_idle_foreground_secs, 180);
    }
}
