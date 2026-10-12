//! Which agent serves an auth domain and which login a pane offers. Pure:
//! values in, values out.

use super::*;

/// The login command for `requested`, tried in order: the active agent
/// (keeps the user's own pinned adapter version when it already signs into
/// that domain), the first catalog entry that does, then the built-in
/// adapter for the domain — so a catalog holding no agent for a domain can
/// still add an account there.
///
/// Total: every candidate that can win is a local `Raw` launch (a domain is
/// only derived from one), and the built-in backstop is too, so
/// `local_command` always answers. A domain with no local login flow at all
/// would make this fallible again.
pub(in crate::workspace) fn resolve_login_cli(
    agents: &[daruda_config::AgentDefinition],
    active_id: &str,
    requested: AccountRecipeId,
) -> AdapterCliRequest {
    resolve_domain_cli(
        agents,
        active_id,
        requested,
        recipe_for(requested).login_args(),
    )
}

/// The command that asks `requested`'s CLI for its auth status, resolved the
/// same way [`resolve_login_cli`] resolves the login — so the status read
/// and the login it describes always come from one adapter.
///
/// `None` for a domain with no confirmed status command
/// ([`AccountRecipe::status_probe`](daruda_agent::accounts::AccountRecipe::status_probe)).
pub(in crate::workspace) fn resolve_status_cli(
    agents: &[daruda_config::AgentDefinition],
    active_id: &str,
    requested: AccountRecipeId,
) -> Option<(
    AdapterCliRequest,
    daruda_agent::accounts::auth_status::AuthStatusFormat,
)> {
    let probe = recipe_for(requested).status_probe()?;
    Some((
        resolve_domain_cli(agents, active_id, requested, probe.args),
        probe.format,
    ))
}

/// Whether `accounts` or `agents` give this user a stake in `recipe`'s domain.
///
/// Pure so the gate that keeps a passive refresh from downloading an adapter is
/// testable without a `Workspace`.
pub(in crate::workspace) fn stake_in_domain(
    accounts: &AccountsState,
    agents: &[daruda_config::AgentDefinition],
    recipe: AccountRecipeId,
) -> bool {
    accounts.accounts.iter().any(|a| a.recipe == recipe)
        || agents
            .iter()
            .any(|a| a.launch.account_recipe(false) == Some(recipe))
}

/// The adapter launch that serves `requested`'s auth domain, paired with the
/// CLI `args` to run on it.
fn resolve_domain_cli(
    agents: &[daruda_config::AgentDefinition],
    active_id: &str,
    requested: AccountRecipeId,
    args: &[&str],
) -> AdapterCliRequest {
    // No lane in scope for this catalog-wide scan — `is_remote: false` is
    // not a guess, it's the honest answer: a bare `Raw` command can't
    // self-report as remote-only (see `account_recipe`'s doc), so this
    // flow's exclusions stay limited to what `Ssh`/`Docker`/`{{cwd}}`
    // already self-describe, exactly as before this axis moved to the lane.
    let signs_into_requested =
        |a: &&daruda_config::AgentDefinition| a.launch.account_recipe(false) == Some(requested);
    let active = agents
        .iter()
        .find(|a| a.id == active_id)
        .filter(signs_into_requested);
    let from_catalog = || agents.iter().find(signs_into_requested);
    let agent = active
        .or_else(from_catalog)
        .map(AgentLaunchSpec::of)
        .unwrap_or_else(|| AgentLaunchSpec::of(&builtin_agent_for(requested)));
    let adapter = agent
        .launch
        .local_command()
        .expect("a domain-deriving launch is local Raw, which always has a local command")
        .to_owned();
    AdapterCliRequest {
        adapter,
        agent_env: agent.env,
        words: args.iter().map(|word| (*word).to_owned()).collect(),
    }
}

/// The built-in agent for `recipe` — the fallback when the user's catalog has
/// no agent for that auth domain.
fn builtin_agent_for(recipe: AccountRecipeId) -> daruda_config::AgentDefinition {
    match recipe {
        AccountRecipeId::Claude => daruda_config::AgentDefinition::claude_default(),
        AccountRecipeId::Codex => daruda_config::AgentDefinition::codex_default(),
    }
}

/// The login a pane's "sign in again" button runs, or `None` when the pane
/// has none to offer.
///
/// Two inputs because neither alone is enough: a managed selection names an
/// account but not its auth domain (the row does), and a `SystemDefault`
/// selection names a domain-less choice whose ambient home is *per domain* —
/// only the pane's own agent says which.
pub(in crate::workspace) fn pane_login_target(
    selection: daruda_store::accounts::AccountSelection,
    domain: crate::workspace::main_area::pane::AccountDomain,
    accounts: &AccountsState,
) -> Option<LoginTarget> {
    use crate::workspace::main_area::pane::AccountDomain;
    use daruda_store::accounts::AccountSelection;
    match selection {
        // The account's own domain, not the pane's: signing in with the
        // pane's domain would write another domain's credentials into this
        // account's dir.
        AccountSelection::Managed(id) => accounts.find(id).map(|account| LoginTarget::Managed {
            id,
            recipe: account.recipe,
        }),
        AccountSelection::SystemDefault => match domain {
            AccountDomain::Exactly(recipe) => Some(LoginTarget::System { recipe }),
            AccountDomain::Any | AccountDomain::Unsupported => None,
        },
    }
}
