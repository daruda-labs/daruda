//! Managed auth-domain accounts: per-account isolated config dir + defaults.
//! Persisted as `accounts.json`; runtime injection/identity lives in
//! `daruda_claude::accounts` (this module is pure data).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod persistence;
pub use persistence::{load_accounts, load_accounts_in, save_accounts, save_accounts_in};

/// Bump when `AccountsState`'s shape changes incompatibly.
pub const SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccountRecipeId {
    #[default]
    Claude,
    Codex,
}

impl AccountRecipeId {
    /// Every auth domain, in the order the UI lists them — the single place
    /// to extend when a domain is added, so no call site hand-rolls its own
    /// array.
    pub const ALL: [AccountRecipeId; 2] = [Self::Claude, Self::Codex];
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

/// A pane's account choice. `SystemDefault` is the explicit "System
/// (`~/.claude`)" selection — ambient environment, no `CLAUDE_CONFIG_DIR`
/// override; `Managed(id)` pins the pane to a managed account's isolated
/// config dir.
///
/// Replaces the former overloaded `Option<AccountId>`, whose inner `None`
/// meant *both* "unset → fall back to the domain default" *and* "explicit
/// system default" — an ambiguity that produced three separate
/// account-switching bugs. There is no "unset" state here: a fresh pane is
/// seeded with an explicit selection at creation, so resolution never needs
/// a domain-default fallback (the fallback was the bug).
///
/// Persisted as `Option<AccountId>` (`None` ↔ `SystemDefault`, `Some(id)` ↔
/// `Managed(id)`) via [`Self::from_persisted`] / [`Self::to_persisted`], so
/// the on-disk schema is unchanged and no migration is required.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum AccountSelection {
    #[default]
    SystemDefault,
    Managed(AccountId),
}

impl AccountSelection {
    /// Reconstruct a runtime selection from the persisted `Option<AccountId>`.
    pub fn from_persisted(value: Option<AccountId>) -> Self {
        match value {
            Some(id) => Self::Managed(id),
            None => Self::SystemDefault,
        }
    }

    /// The persisted representation (`None` for the system default).
    pub fn to_persisted(self) -> Option<AccountId> {
        match self {
            Self::Managed(id) => Some(id),
            Self::SystemDefault => None,
        }
    }

    /// The managed account id this selection pins, or `None` for the
    /// system default. (Same value as [`Self::to_persisted`]; named for
    /// call sites that read it as "which account", not "how to persist".)
    pub fn account_id(self) -> Option<AccountId> {
        self.to_persisted()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManagedAccount {
    pub id: AccountId,
    #[serde(rename = "provider")]
    pub recipe: AccountRecipeId,
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
    /// Recipe → "default account for new panes". Empty means every new pane
    /// starts on the system default; only an explicit user choice fills it.
    /// Deliberately **not** aliased to v1's `default_by_provider` key — see
    /// [`persistence::load_accounts_in`]'s migration.
    #[serde(default)]
    pub default_by_recipe: HashMap<AccountRecipeId, AccountId>,
    #[serde(default)]
    pub schema_version: u32,
}

impl Default for AccountsState {
    fn default() -> Self {
        Self {
            accounts: Vec::new(),
            default_by_recipe: HashMap::new(),
            schema_version: SCHEMA_VERSION,
        }
    }
}

impl AccountsState {
    /// Resolve the account a new pane should use for `recipe`
    /// (the configured default). `None` when no default is set.
    pub fn default_account(&self, recipe: AccountRecipeId) -> Option<&ManagedAccount> {
        let id = self.default_by_recipe.get(&recipe)?;
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
            recipe: AccountRecipeId::Claude,
            email: Some("alice@company.com".into()),
            organization: Some("Acme".into()),
            config_dir: std::path::PathBuf::from("/tmp/acc/alice"),
            created_at: 1,
            last_authenticated_at: 2,
        });
        state.default_by_recipe.insert(AccountRecipeId::Claude, id);

        let json = serde_json::to_string(&state).unwrap();
        let back: AccountsState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.accounts.len(), 1);
        assert_eq!(back.accounts[0].email.as_deref(), Some("alice@company.com"));
        assert_eq!(
            back.default_by_recipe.get(&AccountRecipeId::Claude),
            Some(&id)
        );
        // The Rust field is `recipe`, but the wire key stays `"provider"` —
        // no `accounts.json` migration needed for this rename.
        assert!(json.contains("\"provider\":\"claude\""));
    }

    #[test]
    fn account_recipe_id_defaults_to_claude() {
        assert_eq!(AccountRecipeId::default(), AccountRecipeId::Claude);
    }

    #[test]
    fn all_lists_every_recipe_in_display_order() {
        // The match is exhaustive, so a new variant is a compile error right
        // here next to `ALL`'s own list rather than a domain that silently
        // never reaches the UI.
        let names: Vec<&str> = AccountRecipeId::ALL
            .iter()
            .map(|id| match id {
                AccountRecipeId::Claude => "claude",
                AccountRecipeId::Codex => "codex",
            })
            .collect();
        assert_eq!(names, ["claude", "codex"]);
    }
}
