//! `[logs]` section — disk retention rules for the observability log
//! writer (`~/.daruda/logs/<profile>/`).
//!
//! Two knobs controlling how long error reports stay on disk:
//!
//! - `retention_days` — files older than this are pruned at startup
//!   and once per day. Default 30.
//! - `max_file_size_mb` — when the active daily file reaches this size,
//!   the writer rolls to `daruda-YYYY-MM-DD.001.log`,
//!   `…002.log`, … so a busy session can't grow a single file
//!   unbounded. Default 10 MB.
//!
//! Both fields tolerate `0`, meaning "disabled". `retention_days = 0`
//! keeps every file forever; `max_file_size_mb = 0` disables size-based
//! rolling and the writer keeps appending to one file per day.

use serde::{Deserialize, Serialize};

/// Default age cap (days) before a log file is pruned.
pub const DEFAULT_RETENTION_DAYS: u32 = 30;
/// Default per-file size cap (MB) before rolling to the next ordinal.
pub const DEFAULT_MAX_FILE_SIZE_MB: u32 = 10;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LogsConfig {
    /// Number of days to retain log files. Files with mtime older than
    /// this are deleted at startup and once per day. `0` disables the
    /// age check (files accumulate forever).
    pub retention_days: u32,
    /// Per-file size cap in megabytes. When the active daily file
    /// reaches this size the writer rolls to the next ordinal
    /// (`daruda-YYYY-MM-DD.001.log`, `…002.log`, …). `0` disables the
    /// size cap (one file per day, unbounded).
    pub max_file_size_mb: u32,
}

impl Default for LogsConfig {
    fn default() -> Self {
        Self {
            retention_days: DEFAULT_RETENTION_DAYS,
            max_file_size_mb: DEFAULT_MAX_FILE_SIZE_MB,
        }
    }
}

impl LogsConfig {
    /// `retention_days` as a `Duration`. Returns `None` when retention
    /// is disabled (`0`).
    pub fn retention_duration(&self) -> Option<std::time::Duration> {
        if self.retention_days == 0 {
            None
        } else {
            Some(std::time::Duration::from_secs(
                u64::from(self.retention_days) * 24 * 60 * 60,
            ))
        }
    }

    /// `max_file_size_mb` as bytes. Returns `None` when size rolling
    /// is disabled (`0`).
    pub fn max_file_size_bytes(&self) -> Option<u64> {
        if self.max_file_size_mb == 0 {
            None
        } else {
            Some(u64::from(self.max_file_size_mb) * 1024 * 1024)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_30d_10mb() {
        let cfg = LogsConfig::default();
        assert_eq!(cfg.retention_days, 30);
        assert_eq!(cfg.max_file_size_mb, 10);
        assert_eq!(
            cfg.retention_duration().unwrap().as_secs(),
            30 * 24 * 60 * 60
        );
        assert_eq!(cfg.max_file_size_bytes(), Some(10 * 1024 * 1024));
    }

    #[test]
    fn zero_disables_each_knob_independently() {
        let cfg = LogsConfig {
            retention_days: 0,
            max_file_size_mb: 5,
        };
        assert!(cfg.retention_duration().is_none());
        assert_eq!(cfg.max_file_size_bytes(), Some(5 * 1024 * 1024));

        let cfg = LogsConfig {
            retention_days: 7,
            max_file_size_mb: 0,
        };
        assert_eq!(
            cfg.retention_duration().unwrap().as_secs(),
            7 * 24 * 60 * 60
        );
        assert!(cfg.max_file_size_bytes().is_none());
    }

    #[test]
    fn parses_partial_section() {
        // Only retention_days set — max_file_size_mb falls back to default.
        let toml = "retention_days = 7";
        let parsed: LogsConfig = toml::from_str(toml).expect("parse");
        assert_eq!(parsed.retention_days, 7);
        assert_eq!(parsed.max_file_size_mb, DEFAULT_MAX_FILE_SIZE_MB);
    }
}
