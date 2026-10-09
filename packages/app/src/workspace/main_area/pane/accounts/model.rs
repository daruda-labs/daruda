//! Account domains, selection, and environment resolution without GPUI.

use crate::agent::account::PreparedAccount;
use daruda_terminal::pty::PtyConfig;
use std::path::{Path, PathBuf};

/// Which auth domains a pane may resolve a managed account from. Three
/// distinct states, not an `Option<AccountRecipeId>`: "any domain" and "no
/// domain at all" are opposites, and collapsing both onto `None` let a pane
/// with no derivable domain accept an account from every domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum AccountDomain {
    /// No agent (a terminal shell, which may run any CLI): every managed
    /// account is usable, and the account's own recipe decides which env var
    /// carries its dir.
    Any,
    /// The pane's agent signs into exactly this domain.
    Exactly(daruda_store::accounts::AccountRecipeId),
    /// The pane's agent has no local auth domain — a remote launch, a JSON
    /// stdio config, or an adapter daruda doesn't recognize — so it can hold
    /// no managed account.
    Unsupported,
}

/// An account-carrying pane as the account layer sees it. File / TaskEdit
/// panes track no account and are never described this way.
pub(in crate::workspace) enum AccountPane {
    Terminal,
    /// Agent chat, carrying the launch of the agent it runs (`None` when that
    /// agent id is no longer in the catalog) and whether *this pane's* lane
    /// is currently remote — a `Raw` launch carries no host of its own, so
    /// the domain gate below can't tell local from remote by the launch
    /// alone (see `AgentLaunch::account_recipe`'s doc).
    AgentChat {
        launch: Option<daruda_config::AgentLaunch>,
        is_remote: bool,
    },
}

impl AccountDomain {
    /// The domain an agent-chat pane resolves in, from its adapter's derived
    /// auth domain. No domain means no managed account, never "any".
    pub(in crate::workspace) fn for_agent(
        recipe: Option<daruda_store::accounts::AccountRecipeId>,
    ) -> Self {
        match recipe {
            Some(r) => Self::Exactly(r),
            None => Self::Unsupported,
        }
    }

    /// The domain a pane resolves in. An agent-chat pane is scoped to *its
    /// own* agent's domain rather than the session's active agent, so a Codex
    /// pane never offers Claude accounts.
    ///
    /// A deprecated `Ssh`/`Docker` launch that `is_remote` reports as
    /// currently resolving `Local` (the lane's Session Host picked Local,
    /// retiring the launch's own embedded host) is special-cased the same
    /// way the connect path does — `account_recipe()` alone can't see past
    /// the launch's own shape, but `is_remote` already reflects the lane's
    /// verified locality here, so the recipe is derived from the bare
    /// adapter command directly (mirrors
    /// `crate::agent::launch_resolve::account_recipe_for_connect`, which the
    /// account-switcher display and the actual connect must agree with —
    /// otherwise the switcher would keep hiding accounts the connect can
    /// now use).
    pub(in crate::workspace) fn for_pane(pane: &AccountPane) -> Self {
        match pane {
            AccountPane::Terminal => Self::Any,
            AccountPane::AgentChat { launch, is_remote } => {
                let recipe = match launch {
                    Some(
                        daruda_config::AgentLaunch::Ssh {
                            adapter_command, ..
                        }
                        | daruda_config::AgentLaunch::Docker {
                            adapter_command, ..
                        },
                    ) if !is_remote => {
                        daruda_config::account_recipe_for_local_command(adapter_command)
                    }
                    Some(l) => l.account_recipe(*is_remote),
                    None => None,
                };
                Self::for_agent(recipe)
            }
        }
    }
}

/// Inject the account's config-dir env var and remove the auth-override vars
/// its recipe strips, so OAuth account selection wins (see
/// `daruda_config::account_env`).
pub(in crate::workspace::main_area::pane) fn apply_account_env(
    pty: &mut PtyConfig,
    env: &daruda_config::AccountEnv,
) {
    pty.env.retain(|(k, _)| !env.strip.contains(&k.as_str()));
    pty.env.extend(env.inject.iter().cloned());
}

/// Resolve a pane's account selection into the config dir + env to spawn
/// with. Pure (no I/O) so the domain gate stays unit-testable; preparing the
/// dir is a separate, explicit step at the call site.
///
/// `domain` is what the pane's process can sign into — see [`AccountDomain`].
/// `SystemDefault` resolves to `None` with no domain-default fallback: that
/// fallback overrode an explicit "System" choice, so the default is seeded at
/// pane creation instead.
pub(in crate::workspace) fn resolve_pane_account(
    state: &daruda_store::accounts::AccountsState,
    data_dir: &Path,
    selection: daruda_store::accounts::AccountSelection,
    domain: AccountDomain,
) -> Option<PreparedAccount> {
    if domain == AccountDomain::Unsupported {
        return None;
    }
    let id = selection.account_id()?;
    let account = state.find(id)?;
    if matches!(domain, AccountDomain::Exactly(r) if account.recipe != r) {
        return None;
    }
    let recipe = daruda_agent::accounts::recipe_for(account.recipe);
    let config_dir = daruda_agent::accounts::account_config_dir(data_dir, id);
    let env = daruda_config::account_env(recipe.config_dir_env(), &config_dir, recipe.strip_env());
    Some(PreparedAccount {
        recipe: account.recipe,
        config_dir,
        env,
    })
}

/// The focused pane's resolved account: the usage-cache key, the auth
/// domain, and the config dir as one value, so a caller can't pair a dir
/// with the wrong domain or cache under a key nothing resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum FocusedAccount {
    /// No managed account — the ambient environment.
    SystemDefault,
    Managed {
        id: daruda_store::accounts::AccountId,
        recipe: daruda_store::accounts::AccountRecipeId,
        config_dir: PathBuf,
    },
}

impl FocusedAccount {
    /// Key the per-account usage caches are stored under.
    pub(in crate::workspace) fn key(&self) -> daruda_store::accounts::AccountSelection {
        match self {
            Self::SystemDefault => daruda_store::accounts::AccountSelection::SystemDefault,
            Self::Managed { id, .. } => daruda_store::accounts::AccountSelection::Managed(*id),
        }
    }

    /// The account's isolated config dir, ready to move into a background
    /// task; `None` for the ambient default.
    pub(in crate::workspace) fn into_config_dir(self) -> Option<PathBuf> {
        match self {
            Self::SystemDefault => None,
            Self::Managed { config_dir, .. } => Some(config_dir),
        }
    }
}

/// Pure core of [`Workspace::focused_account`]. A `Managed` id that no
/// longer resolves collapses to [`FocusedAccount::SystemDefault`] rather
/// than caching under a dangling account.
///
/// Passes `required: None` — this is `&self`-only and a pane's agent id
/// lives in the `AgentChatView` entity, so its auth domain isn't reachable.
pub(in crate::workspace::main_area::pane) fn resolve_focused_account(
    selection: daruda_store::accounts::AccountSelection,
    state: &daruda_store::accounts::AccountsState,
    data_dir: &Path,
) -> FocusedAccount {
    match selection.account_id().zip(resolve_pane_account(
        state,
        data_dir,
        selection,
        AccountDomain::Any,
    )) {
        Some((id, account)) => FocusedAccount::Managed {
            id,
            recipe: account.recipe,
            config_dir: account.config_dir,
        },
        None => FocusedAccount::SystemDefault,
    }
}
