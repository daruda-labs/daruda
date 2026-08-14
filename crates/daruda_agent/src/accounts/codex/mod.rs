//! [`CodexRecipe`]: the [`AccountRecipe`] for Codex. Credentials are a
//! plaintext `auth.json` inside the account's `CODEX_HOME` — no Keychain —
//! so the directory alone is the isolation boundary.

pub mod home;
mod identity;

pub use home::system_codex_home;

use std::io;
use std::path::Path;
use std::time::Duration;

use daruda_store::accounts::AccountRecipeId;

use super::identity::AccountIdentity;
use super::login::LoginCompletion;
use super::recipe::{AccountRecipe, CleanupScope, remove_account_dir};

/// Empty by design: codex treats `OPENAI_API_KEY` as a custom-provider
/// selection rather than an override to strip (orca `service.ts:420`).
const AUTH_ENV_STRIP: &[&str] = &[];

/// `cli login` — not `login`. `codex-acp login` resolves the binary as
/// `CODEX_PATH ?? "codex"` and fails without a system install, while
/// `cli login` falls back to the bundled `@openai/codex`.
const LOGIN_ARGS: &str = "cli login";

/// How long `codex login` may linger after writing `auth.json` before it is
/// cancelled. Value taken from orca's
/// `WINDOWS_LOGIN_POST_AUTH_EXIT_GRACE_MS` (`codex-accounts/service.ts:55`);
/// see [`CodexRecipe::login_completion`] for what that does not justify.
const POST_CREDENTIALS_EXIT_GRACE: Duration = Duration::from_secs(5);

pub struct CodexRecipe;

impl AccountRecipe for CodexRecipe {
    fn id(&self) -> AccountRecipeId {
        AccountRecipeId::Codex
    }

    /// `auth.json` landing, not process exit. Orca gates the same grace to
    /// `win32` (`service.ts:1134`) for a Windows-only file-handle problem, so
    /// applying it here is an untested extrapolation — harmless because
    /// `LoginProcess::wait` polls exit first each tick.
    fn login_completion(&self) -> LoginCompletion {
        LoginCompletion::OnCredentials {
            grace: POST_CREDENTIALS_EXIT_GRACE,
        }
    }

    fn config_dir_env(&self) -> &'static str {
        "CODEX_HOME"
    }

    fn strip_env(&self) -> &'static [&'static str] {
        AUTH_ENV_STRIP
    }

    fn login_args(&self) -> &'static str {
        LOGIN_ARGS
    }

    /// Unconfirmed: the codex CLI's status command and its output shape have
    /// not been captured, and a guess would read as "signed out".
    fn status_args(&self) -> Option<&'static str> {
        None
    }

    fn system_home_hint(&self) -> &'static str {
        home::SYSTEM_HOME_HINT
    }

    fn system_home_dir(&self) -> Option<std::path::PathBuf> {
        home::system_codex_home()
    }

    fn prepare_dir(&self, dir: &Path) -> io::Result<()> {
        home::prepare_codex_home(dir)
    }

    fn read_identity(&self, dir: &Path) -> AccountIdentity {
        identity::read_codex_identity(dir)
    }

    fn has_credentials(&self, dir: &Path) -> bool {
        identity::has_codex_credentials(dir)
    }

    /// Credentials are a plaintext `auth.json` inside `dir`, so removing the
    /// dir is the whole of it.
    fn cleanup_scope(&self) -> CleanupScope {
        CleanupScope::DirScoped
    }

    /// INVARIANT: `dir` holds symlinks into the user's real `~/.codex`, and
    /// [`remove_account_dir`] unlinks them rather than following them. Any
    /// replacement must keep that property or it deletes user data.
    fn cleanup(&self, dir: &Path) {
        remove_account_dir(
            dir,
            "Failed to remove Codex account home",
            "account.codex.cleanup_dir_failed",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_removes_the_account_dir_but_not_the_symlink_targets() {
        let source = tempfile::tempdir().expect("source");
        std::fs::create_dir_all(source.path().join("skills")).expect("skills");
        std::fs::write(source.path().join("skills").join("a.md"), b"skill").expect("skill file");
        let dir = tempfile::tempdir().expect("dest").keep();
        std::os::unix::fs::symlink(source.path().join("skills"), dir.join("skills"))
            .expect("symlink");

        CodexRecipe.cleanup(&dir);

        assert!(!dir.exists());
        assert_eq!(
            std::fs::read(source.path().join("skills").join("a.md")).expect("source survives"),
            b"skill"
        );
    }

    #[test]
    fn cleanup_is_a_noop_when_dir_is_already_gone() {
        let dir = std::env::temp_dir().join("daruda-codex-recipe-missing-xyz");
        CodexRecipe.cleanup(&dir);
    }
}
