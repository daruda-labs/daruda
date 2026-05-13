//! Right-panel Usage tab configuration: token-pricing constants and
//! API poll cadence.
//!
//! Sample `config.toml`:
//!
//! ```toml
//! [usage.pricing]
//! input_per_mtok       = 3.0
//! output_per_mtok      = 15.0
//! cache_read_per_mtok  = 0.30
//! cache_write_per_mtok = 3.75
//!
//! [usage.poll]
//! limits_secs = 300   # 5 min — Anthropic OAuth /api/oauth/usage
//! status_secs = 300   # 5 min — status.claude.com service indicator
//! ```
//!
//! Both poll cadences treat `0` as "disable this endpoint" so users
//! on an air-gapped machine (or anyone uncomfortable with daruda
//! contacting Anthropic on a timer) can opt out cleanly. Positive
//! values are clamped to [`PollConfig::MIN_POLL_SECS`] (60 s) so a
//! typo like `1` cannot accidentally hammer the API.

use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct UsageConfig {
    pub pricing: PricingConfig,
    pub poll: PollConfig,
}

/// Per-million-token prices in USD. Defaults match Sonnet-4.6 list
/// pricing as of 2026-05. Users override when Anthropic publishes a
/// new rate sheet or when their plan has contracted rates.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct PricingConfig {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_write_per_mtok: f64,
}

const DEFAULT_INPUT_PRICE: f64 = 3.0;
const DEFAULT_OUTPUT_PRICE: f64 = 15.0;
const DEFAULT_CACHE_READ_PRICE: f64 = 0.30;
const DEFAULT_CACHE_WRITE_PRICE: f64 = 3.75;

impl Default for PricingConfig {
    fn default() -> Self {
        Self {
            input_per_mtok: DEFAULT_INPUT_PRICE,
            output_per_mtok: DEFAULT_OUTPUT_PRICE,
            cache_read_per_mtok: DEFAULT_CACHE_READ_PRICE,
            cache_write_per_mtok: DEFAULT_CACHE_WRITE_PRICE,
        }
    }
}

/// Background-poll cadence for the two endpoints feeding the Usage
/// tab's gauges and service-status pill.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct PollConfig {
    /// Cadence for `GET /api/oauth/usage` (Anthropic plan-rate
    /// windows). `0` disables polling; positive values are clamped
    /// to [`Self::MIN_POLL_SECS`].
    pub limits_secs: u64,
    /// Cadence for `https://status.claude.com/api/v2/status.json`.
    /// Same `0` / clamp rules as `limits_secs`.
    pub status_secs: u64,
}

const DEFAULT_LIMITS_SECS: u64 = 300;
const DEFAULT_STATUS_SECS: u64 = 300;

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            limits_secs: DEFAULT_LIMITS_SECS,
            status_secs: DEFAULT_STATUS_SECS,
        }
    }
}

impl PollConfig {
    /// Lower bound for any positive poll cadence. Below this would
    /// risk hammering Anthropic; the clamp is policy, not knob.
    pub const MIN_POLL_SECS: u64 = 60;

    /// Plan-rate poll cadence as a [`Duration`]. `None` when polling
    /// is disabled (`limits_secs == 0`).
    pub fn limits_interval(&self) -> Option<Duration> {
        positive_interval(self.limits_secs)
    }

    /// Service-status poll cadence as a [`Duration`]. `None` when
    /// polling is disabled (`status_secs == 0`).
    pub fn status_interval(&self) -> Option<Duration> {
        positive_interval(self.status_secs)
    }
}

/// `Some(clamped)` when `secs > 0`, `None` when `secs == 0` (means
/// "disabled" — distinct from "every minute").
fn positive_interval(secs: u64) -> Option<Duration> {
    (secs > 0).then(|| Duration::from_secs(secs.max(PollConfig::MIN_POLL_SECS)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pricing_defaults_match_sonnet_4_6_list_prices() {
        let p = PricingConfig::default();
        assert_eq!(p.input_per_mtok, 3.0);
        assert_eq!(p.output_per_mtok, 15.0);
        assert_eq!(p.cache_read_per_mtok, 0.30);
        assert_eq!(p.cache_write_per_mtok, 3.75);
    }

    #[test]
    fn poll_defaults_are_five_minutes() {
        let p = PollConfig::default();
        assert_eq!(p.limits_secs, 300);
        assert_eq!(p.status_secs, 300);
    }

    #[test]
    fn limits_interval_returns_none_when_zero() {
        let p = PollConfig {
            limits_secs: 0,
            status_secs: 0,
        };
        assert!(p.limits_interval().is_none());
        assert!(p.status_interval().is_none());
    }

    #[test]
    fn positive_below_min_is_clamped_to_floor() {
        let p = PollConfig {
            limits_secs: 1,
            status_secs: 30,
        };
        assert_eq!(
            p.limits_interval(),
            Some(Duration::from_secs(PollConfig::MIN_POLL_SECS))
        );
        assert_eq!(
            p.status_interval(),
            Some(Duration::from_secs(PollConfig::MIN_POLL_SECS))
        );
    }

    #[test]
    fn positive_at_or_above_min_passes_through() {
        let p = PollConfig {
            limits_secs: 60,
            status_secs: 600,
        };
        assert_eq!(p.limits_interval(), Some(Duration::from_secs(60)));
        assert_eq!(p.status_interval(), Some(Duration::from_secs(600)));
    }

    #[test]
    fn deserializes_empty_toml_to_defaults() {
        let c: UsageConfig = toml::from_str("").unwrap();
        assert_eq!(c, UsageConfig::default());
    }

    #[test]
    fn deserializes_partial_pricing_keeps_other_defaults() {
        let toml = r#"
            [pricing]
            input_per_mtok = 5.0
        "#;
        let c: UsageConfig = toml::from_str(toml).unwrap();
        assert_eq!(c.pricing.input_per_mtok, 5.0);
        assert_eq!(c.pricing.output_per_mtok, DEFAULT_OUTPUT_PRICE);
        assert_eq!(c.poll, PollConfig::default());
    }

    #[test]
    fn deserializes_partial_poll_keeps_other_defaults() {
        let toml = r#"
            [poll]
            limits_secs = 0
        "#;
        let c: UsageConfig = toml::from_str(toml).unwrap();
        assert_eq!(c.poll.limits_secs, 0);
        assert_eq!(c.poll.status_secs, DEFAULT_STATUS_SECS);
        assert_eq!(c.pricing, PricingConfig::default());
    }

    #[test]
    fn round_trips_through_toml() {
        let original = UsageConfig {
            pricing: PricingConfig {
                input_per_mtok: 2.5,
                output_per_mtok: 10.0,
                cache_read_per_mtok: 0.25,
                cache_write_per_mtok: 3.0,
            },
            poll: PollConfig {
                limits_secs: 600,
                status_secs: 0,
            },
        };
        let serialized = toml::to_string(&original).unwrap();
        let back: UsageConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(back, original);
    }
}
