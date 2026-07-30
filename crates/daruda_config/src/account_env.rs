//! Environment a managed account's process must run with: an isolated
//! config-dir env var to inject and the auth-override vars to strip so
//! OAuth account selection actually wins (orca `environment.ts:37-52`).
//! Which var name to inject and which vars to strip is auth-domain-specific
//! (owned by the caller's `AccountRecipe`, e.g.
//! `daruda_agent::accounts::ClaudeRecipe`) — this module only assembles
//! the env override set generically.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountEnv {
    pub inject: Vec<(String, String)>,
    pub strip: Vec<&'static str>,
}

/// Build the env override for one managed account's process: inject
/// `env_name=config_dir` and strip `strip`'s auth-override vars.
pub fn account_env(env_name: &str, config_dir: &Path, strip: &[&'static str]) -> AccountEnv {
    AccountEnv {
        inject: vec![(
            env_name.to_string(),
            config_dir.to_string_lossy().into_owned(),
        )],
        strip: strip.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn injects_config_dir_and_strips_auth_overrides() {
        let strip: &[&str] = &["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"];
        let e = account_env(
            "CLAUDE_CONFIG_DIR",
            Path::new("/data/claude-accounts/alice"),
            strip,
        );
        assert!(
            e.inject
                .iter()
                .any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == "/data/claude-accounts/alice")
        );
        assert!(e.strip.contains(&"ANTHROPIC_API_KEY"));
        assert!(e.strip.contains(&"CLAUDE_CODE_OAUTH_TOKEN"));
    }

    #[test]
    fn strip_list_is_passed_through_verbatim() {
        let e = account_env("SOME_ENV", Path::new("/tmp/x"), &[]);
        assert!(e.strip.is_empty());
    }
}
