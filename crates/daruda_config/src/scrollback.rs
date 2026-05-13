use serde::{Deserialize, Serialize};

/// Scrollback buffer configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ScrollbackConfig {
    /// Maximum rows ghostty_vt allocates for the scrollback buffer.
    /// Applies at session creation; changing this requires reopening panes.
    pub max_rows: usize,
    /// Maximum rows to scan during search. Higher values increase
    /// search coverage but may slow down large buffers.
    pub lines: u32,
}

impl Default for ScrollbackConfig {
    fn default() -> Self {
        Self {
            max_rows: 10_000,
            lines: 10_000,
        }
    }
}
