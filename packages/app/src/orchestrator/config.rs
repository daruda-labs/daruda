//! Turning `[orchestrator]` settings into something startable.
//!
//! Resolution is total: every reason the orchestrator cannot run — switched
//! off, an empty catalog, a named agent that no longer exists — collapses to
//! `None`, so the caller takes one branch instead of four.

use daruda_config::{AgentDefinition, OrchestratorConfig};
use daruda_store::accounts::AccountSelection;

/// A configuration that can actually be started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedOrchestrator {
    pub agent_id: String,
    /// Runtime form, not the persisted `Option<AccountId>`: a pane's account is
    /// an explicit selection, never "unset falling back to a default".
    pub account: AccountSelection,
}

/// Pure core of [`resolve`], so the catalog can be a fixture in tests.
pub(crate) fn resolve_from(
    cfg: &OrchestratorConfig,
    catalog: &[AgentDefinition],
) -> Option<ResolvedOrchestrator> {
    if !cfg.enabled {
        return None;
    }
    let agent_id = match cfg.agent_id.as_deref() {
        // An agent that vanished from the catalog is a configuration error,
        // not a reason to silently run under a different one.
        Some(named) => catalog.iter().find(|a| a.id == named)?.id.clone(),
        None => catalog.first()?.id.clone(),
    };
    Some(ResolvedOrchestrator {
        agent_id,
        account: AccountSelection::from_persisted(cfg.account_id),
    })
}

/// The live configuration's answer. `resolved_agents` is the same catalog a
/// chat pane launches from, so the orchestrator cannot resolve an agent no
/// pane could.
pub(crate) fn resolve(cx: &gpui::App) -> Option<ResolvedOrchestrator> {
    let config = crate::settings_store::SettingsStore::global(cx).user_arc();
    resolve_from(&config.orchestrator, &config.resolved_agents())
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_config::AgentLaunch;

    fn cfg(enabled: bool, agent: Option<&str>) -> OrchestratorConfig {
        OrchestratorConfig {
            enabled,
            agent_id: agent.map(str::to_owned),
            account_id: None,
        }
    }

    fn agent(id: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_owned(),
            name: id.to_owned(),
            launch: AgentLaunch::Raw(format!("{id} --acp")),
            ..AgentDefinition::claude_default()
        }
    }

    #[test]
    fn disabled_resolves_to_nothing() {
        let catalog = vec![agent("claude-acp")];
        assert!(resolve_from(&cfg(false, Some("claude-acp")), &catalog).is_none());
    }

    #[test]
    fn an_unset_agent_falls_back_to_the_first_catalog_entry() {
        let catalog = vec![agent("claude-acp"), agent("codex-acp")];
        let resolved = resolve_from(&cfg(true, None), &catalog).expect("resolved");
        assert_eq!(resolved.agent_id, "claude-acp");
    }

    #[test]
    fn a_named_agent_is_honoured_over_the_first_entry() {
        let catalog = vec![agent("claude-acp"), agent("codex-acp")];
        let resolved = resolve_from(&cfg(true, Some("codex-acp")), &catalog).expect("resolved");
        assert_eq!(resolved.agent_id, "codex-acp");
    }

    #[test]
    fn a_named_agent_missing_from_the_catalog_resolves_to_nothing() {
        let catalog = vec![agent("claude-acp")];
        assert!(resolve_from(&cfg(true, Some("gone")), &catalog).is_none());
    }

    #[test]
    fn an_empty_catalog_resolves_to_nothing() {
        assert!(resolve_from(&cfg(true, None), &[]).is_none());
    }

    #[test]
    fn an_account_id_becomes_a_managed_selection_and_none_the_system_default() {
        let catalog = vec![agent("claude-acp")];
        assert_eq!(
            resolve_from(&cfg(true, None), &catalog)
                .expect("resolved")
                .account,
            AccountSelection::SystemDefault
        );

        let id = daruda_store::accounts::AccountId::new();
        let pinned = OrchestratorConfig {
            account_id: Some(id),
            ..cfg(true, None)
        };
        assert_eq!(
            resolve_from(&pinned, &catalog).expect("resolved").account,
            AccountSelection::Managed(id)
        );
    }
}
