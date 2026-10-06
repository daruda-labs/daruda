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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_section_keeps_every_default() {
        let parsed: GitConfig = toml::from_str("").unwrap();
        assert!(parsed.confirm_commit && parsed.confirm_push && parsed.default_commit_message);
    }

    #[test]
    fn each_key_turns_off_on_its_own() {
        let parsed: GitConfig = toml::from_str("confirm_push = false").unwrap();
        assert!(parsed.confirm_commit);
        assert!(!parsed.confirm_push);
        assert!(parsed.default_commit_message);
    }
}
