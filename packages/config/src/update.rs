use serde::{Deserialize, Serialize};

/// Auto-update behavior.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct UpdateConfig {
    /// Whether to check GitHub Releases for a newer version once on
    /// startup. The manual "Check for updates" button is unaffected.
    pub auto_check: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self { auto_check: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_auto_check_is_true() {
        assert!(UpdateConfig::default().auto_check);
    }
}
