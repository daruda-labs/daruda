use serde::{Deserialize, Serialize};

/// `[git]` — the Git changes panel's confirmations and defaults.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct GitConfig {
    /// Ask before a commit lands. Amend always asks: it rewrites history.
    pub confirm_commit: bool,
    /// Ask before a push.
    pub confirm_push: bool,
    /// An empty commit message commits as `Update <file>` / `Update N
    /// files` instead of being refused.
    pub default_commit_message: bool,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            confirm_commit: true,
            confirm_push: true,
            default_commit_message: true,
        }
    }
}
