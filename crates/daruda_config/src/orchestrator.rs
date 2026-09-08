use serde::{Deserialize, Serialize};

use daruda_store::accounts::AccountId;

/// Orchestrator settings — the resident agent a `/daruda <text>` command is
/// handed to.
///
/// Only what the user chooses lives here. Whether the orchestrator is
/// Runtime window state lives in the app registry, not in persisted config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct OrchestratorConfig {
    /// Master switch. Off by default so no window, agent process, or slice of
    /// the account's rate limit is spent until the user opts in.
    pub enabled: bool,
    /// Which catalog agent to run. `None` follows the catalog's first entry,
    /// so a fresh install works without naming one; a name the catalog no
    /// longer holds is a configuration error rather than a reason to silently
    /// pick a different agent.
    pub agent_id: Option<String>,
    /// Managed account to pin the session to. `None` is the system default
    /// (ambient environment, no config-dir override) — the same persisted
    /// encoding as a pane's own choice, so
    /// `AccountSelection::from_persisted` is the one conversion.
    pub account_id: Option<AccountId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_inert() {
        let cfg = OrchestratorConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.agent_id, None);
        assert_eq!(cfg.account_id, None);
    }

    #[test]
    fn toml_round_trip_preserves_every_field() {
        let id = AccountId::new();
        let cfg = OrchestratorConfig {
            enabled: true,
            agent_id: Some("claude-acp".to_owned()),
            account_id: Some(id),
        };
        let serialized = toml::to_string(&cfg).expect("serialize");
        let back: OrchestratorConfig = toml::from_str(&serialized).expect("deserialize");
        assert_eq!(back, cfg);
    }

    #[test]
    fn unspecified_fields_fall_back_to_defaults() {
        let cfg: OrchestratorConfig = toml::from_str("enabled = true\n").expect("deserialize");
        assert!(cfg.enabled);
        assert_eq!(cfg.agent_id, None);
        assert_eq!(cfg.account_id, None);
    }
}
