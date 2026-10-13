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
    /// CLI words run on the prepared adapter for this domain's headless login.
    fn login_args(&self) -> &'static [&'static str];
    /// How to ask this domain's CLI how the user signed in — arguments plus the
    /// shape of what it prints, for [`super::auth_status::read_auth_status`].
    ///
    /// `None` for a domain whose command is not *confirmed*. Guessing one would
    /// spawn a process that prints an error, and the reading would come back
    /// empty in a way indistinguishable from "signed out".
    fn status_probe(&self) -> Option<super::auth_status::AuthStatusProbe>;
    /// The ambient (unmanaged) home this domain reads when no account is
    /// pinned, as a tilde path for display next to the "System" choice.
    fn system_home_hint(&self) -> &'static str;
    /// The same home as a real path, honouring the domain's own config-dir
    /// override env var. A system login writes here instead of into a minted
    /// config dir.
    ///
    /// `None` only when there is no home directory and no override — nothing
    /// to read or write. [`Self::system_home_hint`] is display copy and cannot
    /// stand in: it is a tilde string, not a path.
    ///
    /// NOT automatically a valid argument to [`Self::has_credentials`] — see
    /// that method. A domain whose login completes on credentials landing can
    /// probe this path only if its credentials actually live under it.
    fn system_home_dir(&self) -> Option<std::path::PathBuf>;
    /// Best-effort prep run against `dir` before *any* process spawns under it
    /// — an agent session or a plain shell — e.g. mirroring shared config in.
    fn prepare_dir(&self, dir: &Path) -> std::io::Result<()>;
    fn read_identity(&self, dir: &Path) -> AccountIdentity;
    /// Whether a *managed* account's credentials are present under `dir`.
    ///
    /// Scoped to `dir`, which for Claude means a Keychain item whose service
    /// name is derived from the path — so passing
    /// [`Self::system_home_dir`] answers `false` even for a healthy ambient
    /// login, whose item is the unscoped one the CLI writes. Codex has no such
    /// split (its credentials are a file inside the home), which is why the
    /// system flow can probe it and Claude's cannot.
    ///
    /// Claude only escapes this today because its login completes on process
    /// exit and never probes. Verifying an ambient login needs a system-scoped
    /// read, not this.
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
        daruda_core::path::symlink(source.path(), dir.join("linked")).expect("symlink");

        remove_account_dir(&dir, "test", "test.remove_dir");

        assert!(!dir.exists());
        assert_eq!(
            std::fs::read(source.path().join("real.md")).expect("target survives"),
            b"user data"
        );
    }
}
