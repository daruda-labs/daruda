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
