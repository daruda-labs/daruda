//! Environment a managed account's process must run with: the isolated
//! `CLAUDE_CONFIG_DIR` to inject, and the auth-override vars to strip so
//! OAuth selection actually wins (orca `environment.ts:37-52`).

use std::path::Path;

/// Auth-carrying env vars that override OAuth account selection and must be
/// removed from every spawned account process.
pub const AUTH_ENV_STRIP: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "AWS_BEARER_TOKEN_BEDROCK",
    "ANTHROPIC_CUSTOM_HEADERS",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountEnv {
    pub inject: Vec<(String, String)>,
    pub strip: Vec<&'static str>,
}

pub fn account_env(config_dir: &Path) -> AccountEnv {
    AccountEnv {
        inject: vec![(
            "CLAUDE_CONFIG_DIR".to_string(),
            config_dir.to_string_lossy().into_owned(),
        )],
        strip: AUTH_ENV_STRIP.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn injects_config_dir_and_strips_auth_overrides() {
        let e = account_env(Path::new("/data/claude-accounts/alice"));
        assert!(
            e.inject
                .iter()
                .any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == "/data/claude-accounts/alice")
        );
        assert!(e.strip.contains(&"ANTHROPIC_API_KEY"));
        assert!(e.strip.contains(&"CLAUDE_CODE_OAUTH_TOKEN"));
        assert!(e.strip.contains(&"AWS_BEARER_TOKEN_BEDROCK"));
    }
}
