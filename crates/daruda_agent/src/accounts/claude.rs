//! [`ClaudeRecipe`]: the [`AccountRecipe`] for Claude Code — the only auth
//! domain Plan A/B manage today. Every method delegates to the free
//! function this crate already exposed for it, so this is a structural
//! seam over existing behavior, not new logic.

use std::io;
use std::path::{Path, PathBuf};

use daruda_store::accounts::AccountRecipeId;

use super::credentials::{delete_scoped_credentials, read_scoped_credentials};
use super::identity::{AccountIdentity, read_account_identity};
use super::login::LoginCompletion;
use super::mcp_mirror::mirror_shared_mcp_servers;
use super::recipe::{AccountRecipe, CleanupScope, remove_account_dir};

/// Auth-carrying env vars that override OAuth account selection and must be
/// stripped from every spawned Claude account process.
const AUTH_ENV_STRIP: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "AWS_BEARER_TOKEN_BEDROCK",
    "ANTHROPIC_CUSTOM_HEADERS",
];

/// Suffix appended to an agent's launch command for Claude Code's headless
/// subscription-login flow (see `daruda_config::AgentLaunch::login_command`).
const LOGIN_ARGS: &str = "--cli auth login --claudeai";

/// Suffix that makes the CLI print its auth status as JSON — how the user
/// signed in, which no other source tells daruda (the ACP adapter does not
/// forward it, and a login the user ran themselves left daruda no record).
/// Honours `CLAUDE_CONFIG_DIR`, so it answers for a managed account and the
/// ambient home alike.
const STATUS_ARGS: &str = "--cli auth status --json";

/// System Claude home, relative to `$HOME`. Declared as a macro so the
/// display hint below is concatenated from the same single source.
macro_rules! system_home_dir {
    () => {
        ".claude"
    };
}

const SYSTEM_HOME_DIR: &str = system_home_dir!();

/// [`SYSTEM_HOME_DIR`] as a tilde path, for the Settings "System" choice.
const SYSTEM_HOME_HINT: &str = concat!("~/", system_home_dir!());

/// The ambient Claude home every unmanaged pane reads: `$CLAUDE_CONFIG_DIR`
/// when the user set it, else `~/.claude`. Mirrors
/// [`super::codex::system_codex_home`].
fn system_claude_home() -> Option<PathBuf> {
    system_claude_home_from(std::env::var_os("CLAUDE_CONFIG_DIR"), dirs::home_dir())
}

/// Pure core of [`system_claude_home`], split out so the override and the
/// default can be unit-tested without mutating the real process environment
/// (parallel `cargo test` runs share one).
fn system_claude_home_from(
    override_dir: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(dir) = override_dir {
        return Some(PathBuf::from(dir));
    }
    Some(home?.join(SYSTEM_HOME_DIR))
}

pub struct ClaudeRecipe;

impl AccountRecipe for ClaudeRecipe {
    fn id(&self) -> AccountRecipeId {
        AccountRecipeId::Claude
    }

    /// The Claude CLI exits promptly once the browser flow resolves, and its
    /// credentials land in the Keychain rather than the config dir.
    fn login_completion(&self) -> LoginCompletion {
        LoginCompletion::OnExit
    }

    fn config_dir_env(&self) -> &'static str {
        "CLAUDE_CONFIG_DIR"
    }

    fn strip_env(&self) -> &'static [&'static str] {
        AUTH_ENV_STRIP
    }

    fn login_args(&self) -> &'static str {
        LOGIN_ARGS
    }

    fn status_probe(&self) -> Option<super::auth_status::AuthStatusProbe> {
        Some(super::auth_status::AuthStatusProbe {
            args: STATUS_ARGS,
            format: super::auth_status::AuthStatusFormat::Json,
        })
    }

    fn system_home_hint(&self) -> &'static str {
        SYSTEM_HOME_HINT
    }

    fn system_home_dir(&self) -> Option<PathBuf> {
        system_claude_home()
    }

    fn prepare_dir(&self, dir: &Path) -> io::Result<()> {
        mirror_shared_mcp_servers(dir);
        Ok(())
    }

    fn read_identity(&self, dir: &Path) -> AccountIdentity {
        read_account_identity(dir)
    }

    fn has_credentials(&self, dir: &Path) -> bool {
        read_scoped_credentials(dir).is_ok()
    }

    /// The Keychain item is keyed by `sha256(dir)`, so both halves are scoped
    /// to `dir` alone.
    fn cleanup_scope(&self) -> CleanupScope {
        CleanupScope::DirScoped
    }

    fn cleanup(&self, dir: &Path) {
        delete_scoped_credentials(dir);
        remove_account_dir(
            dir,
            "Failed to remove Claude account config dir",
            "account.claude.cleanup_dir_failed",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_removes_the_config_dir() {
        let dir = tempfile::tempdir().expect("tempdir").keep();
        std::fs::write(dir.join("marker"), b"x").unwrap();
        ClaudeRecipe.cleanup(&dir);
        assert!(!dir.exists());
    }

    #[test]
    fn cleanup_is_a_noop_when_dir_is_already_gone() {
        let dir = std::env::temp_dir().join("daruda-agent-recipe-missing-xyz");
        // Never created — must not panic on a missing dir.
        ClaudeRecipe.cleanup(&dir);
    }

    #[test]
    fn has_credentials_is_false_for_a_dir_with_no_keychain_item() {
        let dir = std::env::temp_dir().join(format!("daruda-recipe-creds-{}", std::process::id()));
        assert!(!ClaudeRecipe.has_credentials(&dir));
    }

    #[test]
    fn the_system_home_follows_the_config_dir_override() {
        assert_eq!(
            system_claude_home_from(
                Some(std::ffi::OsString::from("/elsewhere/claude")),
                Some(PathBuf::from("/home/u"))
            ),
            Some(PathBuf::from("/elsewhere/claude"))
        );
    }

    #[test]
    fn the_system_home_defaults_under_the_home_directory() {
        assert_eq!(
            system_claude_home_from(None, Some(PathBuf::from("/home/u"))),
            Some(PathBuf::from("/home/u/.claude"))
        );
    }

    /// No home and no override means there is nothing to read — the caller
    /// has to refuse the login rather than invent a path.
    #[test]
    fn the_system_home_is_unknown_without_a_home_directory() {
        assert_eq!(system_claude_home_from(None, None), None);
    }

    #[test]
    fn read_identity_parses_the_oauth_account_block() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"a@x.com","organizationName":"Org"}}"#,
        )
        .unwrap();
        let identity = ClaudeRecipe.read_identity(dir.path());
        assert_eq!(identity.email.as_deref(), Some("a@x.com"));
        assert_eq!(identity.organization.as_deref(), Some("Org"));
    }
}
