//! The resolved account a spawn funnel hands to a process: which auth
//! recipe it belongs to, the config directory that carries its state, and
//! the environment variable that points a child at that directory.

use std::path::PathBuf;

/// A managed account resolved for one spawn, with its three parts kept
/// together so a directory can never reach a process under another
/// domain's environment variable.
///
/// INVARIANT: `config_dir` is only a path until a spawn funnel materializes
/// it via `AccountRecipe::prepare_dir`. There are exactly two such funnels —
/// `Workspace::create_pane_with_cwd` for a shell and the agent-chat connect
/// pump — and both prep unconditionally, so no resolve site has to remember
/// to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedAccount {
    pub(crate) recipe: daruda_store::accounts::AccountRecipeId,
    pub(crate) config_dir: PathBuf,
    pub(crate) env: daruda_config::AccountEnv,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The whole point of this module's existence: the type is nameable
    /// from `crate::agent`.
    #[test]
    fn prepared_account_is_constructible_outside_workspace() {
        let account = PreparedAccount {
            recipe: daruda_store::accounts::AccountRecipeId::Claude,
            config_dir: PathBuf::from("/tmp/cfg"),
            // `AccountEnv` has no `Default` impl; both fields are `pub`.
            env: daruda_config::AccountEnv {
                inject: Vec::new(),
                strip: Vec::new(),
            },
        };
        assert_eq!(account.config_dir, PathBuf::from("/tmp/cfg"));
    }
}
