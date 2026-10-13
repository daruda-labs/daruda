//! Right-panel Usage tab configuration: API poll cadence.
//!
//! Sample `config.toml`:
//!
//! ```toml
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
    pub poll: PollConfig,
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
    fn deserializes_partial_poll_keeps_other_defaults() {
        let toml = r#"
            [poll]
            limits_secs = 0
        "#;
        let c: UsageConfig = toml::from_str(toml).unwrap();
        assert_eq!(c.poll.limits_secs, 0);
        assert_eq!(c.poll.status_secs, DEFAULT_STATUS_SECS);
    }
}
