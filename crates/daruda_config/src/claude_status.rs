use serde::{Deserialize, Serialize};

/// Claude Code session-status indicator settings.
///
/// Sizing and animation timings are theme constants (see
/// `daruda_terminal::ux::theme::STATUS_INDICATOR_*`) — only behavioural
/// knobs are exposed via TOML.
///
/// Sample `config.toml`:
///
/// ```toml
/// [claude_status]
/// enable = true
/// stale_threshold_secs = 300   # 5 min: status file age past this resets to Connecting
/// file_ttl_days = 7            # 7 days: cleanup ceiling for orphaned files
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct ClaudeStatusConfig {
    /// Render the indicator on each worktree row. When false, the
    /// hook handler still records status (so toggling back on shows
    /// fresh data immediately) but the left dock skips the indicator
    /// cell entirely.
    pub enable: bool,

    /// Age in seconds past which a status file's `state` is treated as
    /// stale on cold restore — the entry is loaded but its state is
    /// reset to `Connecting`. Subsequent hook events overwrite it
    /// normally. Default: 300 (5 minutes).
    pub stale_threshold_secs: u64,

    /// Age in days past which an orphaned status file is deleted on
    /// cold restore. Catches the case where `SessionEnd` was missed
    /// (Claude crashed, `kill -9`, power loss). Default: 7.
    pub file_ttl_days: u32,

    /// Age in seconds past which a `NeedsAttention` indicator is
    /// downgraded to `Idle` at render time. Catches the case where
    /// Claude Code fired a `Notification` for a permission/idle prompt
    /// that the user dismissed in the TUI without daruda seeing the
    /// follow-up event. Default: 60.
    pub needs_attention_stale_secs: u64,
}

const STALE_THRESHOLD_MIN: u64 = 30; // 30s — anything below this is just noise
const STALE_THRESHOLD_MAX: u64 = 24 * 60 * 60; // 1 day — anything above defeats the point
const FILE_TTL_MIN_DAYS: u32 = 1;
const FILE_TTL_MAX_DAYS: u32 = 365;
const NEEDS_ATTENTION_STALE_MIN: u64 = 10;
const NEEDS_ATTENTION_STALE_MAX: u64 = 3600;

const DEFAULT_STALE_THRESHOLD_SECS: u64 = 300;
const DEFAULT_FILE_TTL_DAYS: u32 = 7;
const DEFAULT_NEEDS_ATTENTION_STALE_SECS: u64 = 60;

impl Default for ClaudeStatusConfig {
    fn default() -> Self {
        Self {
            enable: true,
            stale_threshold_secs: DEFAULT_STALE_THRESHOLD_SECS,
            file_ttl_days: DEFAULT_FILE_TTL_DAYS,
            needs_attention_stale_secs: DEFAULT_NEEDS_ATTENTION_STALE_SECS,
        }
    }
}

impl ClaudeStatusConfig {
    pub fn clamp(&mut self) {
        self.stale_threshold_secs = self
            .stale_threshold_secs
            .clamp(STALE_THRESHOLD_MIN, STALE_THRESHOLD_MAX);
        self.file_ttl_days = self
            .file_ttl_days
            .clamp(FILE_TTL_MIN_DAYS, FILE_TTL_MAX_DAYS);
        self.needs_attention_stale_secs = self
            .needs_attention_stale_secs
            .clamp(NEEDS_ATTENTION_STALE_MIN, NEEDS_ATTENTION_STALE_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = ClaudeStatusConfig::default();
        assert!(c.enable);
        assert_eq!(c.stale_threshold_secs, 300);
        assert_eq!(c.file_ttl_days, 7);
        assert_eq!(c.needs_attention_stale_secs, 60);
    }

    #[test]
    fn clamp_pulls_outliers_in_range() {
        let mut c = ClaudeStatusConfig {
            enable: true,
            stale_threshold_secs: 1,
            file_ttl_days: 0,
            needs_attention_stale_secs: 1,
        };
        c.clamp();
        assert_eq!(c.stale_threshold_secs, STALE_THRESHOLD_MIN);
        assert_eq!(c.file_ttl_days, FILE_TTL_MIN_DAYS);
        assert_eq!(c.needs_attention_stale_secs, NEEDS_ATTENTION_STALE_MIN);

        let mut c = ClaudeStatusConfig {
            enable: true,
            stale_threshold_secs: 10_000_000,
            file_ttl_days: 9999,
            needs_attention_stale_secs: 999_999,
        };
        c.clamp();
        assert_eq!(c.stale_threshold_secs, STALE_THRESHOLD_MAX);
        assert_eq!(c.file_ttl_days, FILE_TTL_MAX_DAYS);
        assert_eq!(c.needs_attention_stale_secs, NEEDS_ATTENTION_STALE_MAX);
    }

    #[test]
    fn deserializes_partial_toml() {
        // Empty section → defaults, missing fields → defaults.
        let c: ClaudeStatusConfig = toml::from_str("").unwrap();
        assert_eq!(c, ClaudeStatusConfig::default());

        let c: ClaudeStatusConfig = toml::from_str("enable = false").unwrap();
        assert!(!c.enable);
        assert_eq!(c.stale_threshold_secs, 300);
    }
}
