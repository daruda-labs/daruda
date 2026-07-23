//! Managed AI-provider accounts: per-account isolated config dir + defaults.
//! Persisted as `accounts.json`; runtime injection/identity lives in
//! `daruda_claude::accounts` (this module is pure data).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod persistence;
pub use persistence::{load_accounts, load_accounts_in, save_accounts, save_accounts_in};

/// Bump when `AccountsState`'s shape changes incompatibly.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentProvider {
    #[default]
    Claude,
    Codex,
}

/// Stable, globally-unique account identifier (see `ProjectUuid` precedent).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(pub Uuid);

impl AccountId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AccountId {
    fn default() -> Self {
        Self(Uuid::nil())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManagedAccount {
    pub id: AccountId,
    pub provider: AgentProvider,
    /// Captured from `oauthAccount` after login (Plan B); `None` when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    /// Per-account isolated `CLAUDE_CONFIG_DIR`.
    pub config_dir: PathBuf,
    pub created_at: u64,
    pub last_authenticated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountsState {
    #[serde(default)]
    pub accounts: Vec<ManagedAccount>,
    /// Provider → "default account for new panes".
    #[serde(default)]
    pub default_by_provider: HashMap<AgentProvider, AccountId>,
    #[serde(default)]
    pub schema_version: u32,
}

impl Default for AccountsState {
    fn default() -> Self {
        Self {
            accounts: Vec::new(),
            default_by_provider: HashMap::new(),
            schema_version: SCHEMA_VERSION,
        }
    }
}

impl AccountsState {
    /// Resolve the account a new pane should use for `provider`
    /// (the configured default). `None` when no default is set.
    pub fn default_account(&self, provider: AgentProvider) -> Option<&ManagedAccount> {
        let id = self.default_by_provider.get(&provider)?;
        self.accounts.iter().find(|a| a.id == *id)
    }

    pub fn find(&self, id: AccountId) -> Option<&ManagedAccount> {
        self.accounts.iter().find(|a| a.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_state_json_round_trips() {
        let id = AccountId::new();
        let mut state = AccountsState::default();
        state.accounts.push(ManagedAccount {
            id,
            provider: AgentProvider::Claude,
            email: Some("alice@company.com".into()),
            organization: Some("Acme".into()),
            config_dir: std::path::PathBuf::from("/tmp/acc/alice"),
            created_at: 1,
            last_authenticated_at: 2,
        });
        state.default_by_provider.insert(AgentProvider::Claude, id);

        let json = serde_json::to_string(&state).unwrap();
        let back: AccountsState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.accounts.len(), 1);
        assert_eq!(back.accounts[0].email.as_deref(), Some("alice@company.com"));
        assert_eq!(
            back.default_by_provider.get(&AgentProvider::Claude),
            Some(&id)
        );
    }

    #[test]
    fn agent_provider_defaults_to_claude() {
        assert_eq!(AgentProvider::default(), AgentProvider::Claude);
    }
}
