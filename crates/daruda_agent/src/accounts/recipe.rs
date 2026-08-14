//! Auth-domain strategy: hides per-account behavior (env var to inject,
//! vars to strip, login args, config-dir prep/identity/credentials/cleanup)
//! behind one interface, keyed by [`AccountRecipeId`]:
//! [`super::claude::ClaudeRecipe`] and [`super::codex::CodexRecipe`].

use std::io;
use std::path::Path;

use daruda_store::accounts::AccountRecipeId;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use super::identity::AccountIdentity;
use super::login::LoginCompletion;

/// How far an [`AccountRecipe::cleanup`] reaches. The startup orphan sweep
/// has no account record to read a domain from, so it may only run a cleanup
/// that is harmless on a dir belonging to some other domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupScope {
    /// Only the dir plus credential-store state keyed by its path: idempotent,
    /// and a no-op against another domain's dir.
    DirScoped,
    /// Reaches past the dir (e.g. revoking a token with a remote service), so
    /// it must never be run speculatively.
    External,
}

pub trait AccountRecipe: Send + Sync {
    fn id(&self) -> AccountRecipeId;
    /// What proves this domain's headless login finished — see
    /// [`LoginCompletion`]; consumed by `LoginProcess::wait`.
    fn login_completion(&self) -> LoginCompletion;
    /// Env var name whose value is the account's isolated config dir
    /// (e.g. `"CLAUDE_CONFIG_DIR"`).
    fn config_dir_env(&self) -> &'static str;
    /// Auth-carrying env vars that must be stripped from a spawned account
    /// process so its own config-dir credentials win over an inherited one.
    fn strip_env(&self) -> &'static [&'static str];
    /// Suffix appended to an agent's launch command to run this domain's
    /// headless login flow.
    fn login_args(&self) -> &'static str;
    /// The ambient (unmanaged) home this domain reads when no account is
    /// pinned, as a tilde path for display next to the "System" choice.
    fn system_home_hint(&self) -> &'static str;
    /// The same home as a real path, honouring the domain's own config-dir
    /// override env var. A system login writes here instead of into a minted
    /// config dir, so this is what [`Self::has_credentials`] gets probed
    /// against for a domain whose login completes on credentials landing.
    ///
    /// `None` only when there is no home directory and no override — nothing
    /// to read or write. [`Self::system_home_hint`] is display copy and cannot
    /// stand in: it is a tilde string, not a path.
    fn system_home_dir(&self) -> Option<std::path::PathBuf>;
    /// Best-effort prep run against `dir` before *any* process spawns under it
    /// — an agent session or a plain shell — e.g. mirroring shared config in.
    fn prepare_dir(&self, dir: &Path) -> std::io::Result<()>;
    fn read_identity(&self, dir: &Path) -> AccountIdentity;
    fn has_credentials(&self, dir: &Path) -> bool;
    /// Remove `dir` and any OS-level credential store entry scoped to it.
    fn cleanup(&self, dir: &Path);
    /// See [`CleanupScope`] — decides whether the orphan sweep may run
    /// [`Self::cleanup`] against a dir it cannot attribute to a domain.
    fn cleanup_scope(&self) -> CleanupScope;
}

/// Resolve the [`AccountRecipe`] for `id`.
pub fn recipe_for(id: AccountRecipeId) -> &'static dyn AccountRecipe {
    crate::providers::integration_for(id).account
}

/// Best-effort removal of a managed account's config dir, shared by every
/// recipe's [`AccountRecipe::cleanup`]. A missing dir is success, and
/// `remove_dir_all` unlinks symlinks rather than following them (Codex plants
/// links into the user's real `~/.codex`). `dedup` names the calling domain,
/// since the report's `at` resolves here rather than to the caller.
pub(super) fn remove_account_dir(dir: &Path, message: &str, dedup: &str) {
    if let Err(e) = std::fs::remove_dir_all(dir)
        && e.kind() != io::ErrorKind::NotFound
    {
        LogWriter::log(
            ErrorReport::new(message)
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("error", format!("{e}"))
                .dedup(dedup)
                .build(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipe_for_claude_matches_current_constants() {
        let recipe = recipe_for(AccountRecipeId::Claude);
        assert_eq!(recipe.id(), AccountRecipeId::Claude);
        assert_eq!(recipe.config_dir_env(), "CLAUDE_CONFIG_DIR");
        assert_eq!(recipe.login_args(), "--cli auth login --claudeai");
        assert_eq!(recipe.system_home_hint(), "~/.claude");
        let strip = recipe.strip_env();
        for var in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "AWS_BEARER_TOKEN_BEDROCK",
            "ANTHROPIC_CUSTOM_HEADERS",
        ] {
            assert!(strip.contains(&var), "missing strip var: {var}");
        }
        assert_eq!(strip.len(), 5);
    }

    #[test]
    fn recipe_for_codex_matches_current_constants() {
        let recipe = recipe_for(AccountRecipeId::Codex);
        assert_eq!(recipe.id(), AccountRecipeId::Codex);
        assert_eq!(recipe.config_dir_env(), "CODEX_HOME");
        assert_eq!(recipe.login_args(), "cli login");
        assert_eq!(recipe.system_home_hint(), "~/.codex");
        assert!(recipe.strip_env().is_empty());
    }

    #[test]
    fn every_recipe_cleans_up_only_dir_scoped_state() {
        // The orphan sweep runs every `DirScoped` cleanup against a dir it
        // can't attribute to a domain, so a cleanup that ever reaches further
        // has to declare `External` here to stay out of that path.
        for id in AccountRecipeId::all() {
            assert_eq!(recipe_for(id).cleanup_scope(), CleanupScope::DirScoped);
        }
    }

    #[test]
    fn remove_account_dir_removes_the_dir_and_tolerates_a_missing_one() {
        let dir = tempfile::tempdir().expect("tempdir").keep();
        std::fs::write(dir.join("marker"), b"x").expect("marker");
        remove_account_dir(&dir, "test", "test.remove_dir");
        assert!(!dir.exists());
        // Second pass has nothing to remove and must stay silent.
        remove_account_dir(&dir, "test", "test.remove_dir");
    }

    #[test]
    fn remove_account_dir_unlinks_symlinks_without_touching_their_targets() {
        let source = tempfile::tempdir().expect("source");
        std::fs::write(source.path().join("real.md"), b"user data").expect("source file");
        let dir = tempfile::tempdir().expect("dest").keep();
        std::os::unix::fs::symlink(source.path(), dir.join("linked")).expect("symlink");

        remove_account_dir(&dir, "test", "test.remove_dir");

        assert!(!dir.exists());
        assert_eq!(
            std::fs::read(source.path().join("real.md")).expect("target survives"),
            b"user data"
        );
    }

    /// A system login runs against the user's own home rather than a minted
    /// config dir, so every domain has to be able to name that home — a domain
    /// that cannot has no way to probe whether its login landed.
    #[test]
    fn every_recipe_can_name_its_system_home() {
        for id in AccountRecipeId::all() {
            let recipe = recipe_for(id);
            let home = recipe
                .system_home_dir()
                .expect("a machine running the tests has a home directory");
            assert!(
                home.is_absolute(),
                "{id:?} yielded a relative home: {home:?}"
            );
        }
    }

    /// The hint is display copy and the dir is a real path, but they must name
    /// the same place — a hint that says `~/.claude` next to a login writing
    /// somewhere else is worse than no hint.
    #[test]
    fn the_system_home_hint_describes_the_system_home_dir() {
        for id in AccountRecipeId::all() {
            let recipe = recipe_for(id);
            // Skip when the user's environment overrides the home — then the
            // hint intentionally still shows the default location.
            if std::env::var_os(recipe.config_dir_env()).is_some() {
                continue;
            }
            let home = recipe.system_home_dir().expect("home directory");
            let hint = recipe.system_home_hint();
            let tail = hint.strip_prefix("~/").expect("hints are tilde paths");
            assert_eq!(
                home.file_name().and_then(std::ffi::OsStr::to_str),
                Some(tail),
                "{id:?}: hint {hint} does not name {home:?}"
            );
        }
    }

    #[test]
    fn claude_completes_a_login_on_process_exit() {
        assert_eq!(
            recipe_for(AccountRecipeId::Claude).login_completion(),
            LoginCompletion::OnExit
        );
    }

    #[test]
    fn codex_completes_a_login_on_credentials_landing() {
        assert!(matches!(
            recipe_for(AccountRecipeId::Codex).login_completion(),
            LoginCompletion::OnCredentials { grace } if grace > std::time::Duration::ZERO
        ));
    }
}
