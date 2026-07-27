//! Status bar Ports segment configuration: local TCP listening-port
//! scan cadence. Unlike [`crate::usage::PollConfig`] this never touches
//! the network — the scan runs `lsof`/`/proc` locally — so there is no
//! "disable via 0" knob; the segment is disabled instead by removing
//! [`crate::StatusBarItem::Ports`] from `StatusBarConfig::visible_items`.

use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct PortsConfig {
    /// Cadence for the listening-port scan. Clamped to
    /// [`Self::MIN_POLL_SECS`] so a typo can't spin a subprocess in a
    /// tight loop.
    pub poll_secs: u64,
}

const DEFAULT_POLL_SECS: u64 = 5;

impl Default for PortsConfig {
    fn default() -> Self {
        Self {
            poll_secs: DEFAULT_POLL_SECS,
        }
    }
}

impl PortsConfig {
    /// Lower bound for the scan cadence.
    pub const MIN_POLL_SECS: u64 = 2;

    /// Scan cadence as a [`Duration`], clamped to [`Self::MIN_POLL_SECS`].
    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.poll_secs.max(Self::MIN_POLL_SECS))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_five_seconds() {
        assert_eq!(PortsConfig::default().poll_secs, DEFAULT_POLL_SECS);
    }

    #[test]
    fn interval_clamps_to_minimum() {
        let cfg = PortsConfig { poll_secs: 1 };
        assert_eq!(
            cfg.interval(),
            Duration::from_secs(PortsConfig::MIN_POLL_SECS)
        );
    }

    #[test]
    fn interval_passes_through_above_minimum() {
        let cfg = PortsConfig { poll_secs: 10 };
        assert_eq!(cfg.interval(), Duration::from_secs(10));
    }

    #[test]
    fn deserializes_empty_toml_to_defaults() {
        let cfg: PortsConfig = toml::from_str("").unwrap();
        assert_eq!(cfg, PortsConfig::default());
    }
}
