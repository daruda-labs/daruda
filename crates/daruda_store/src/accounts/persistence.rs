//! `accounts.json` load/save via the shared JSON helpers.

use std::path::{Path, PathBuf};

use crate::accounts::{AccountsState, SCHEMA_VERSION};
use crate::persistence::{LoadOutcome, default_data_dir, load_json_file, save_json_atomic};

pub fn accounts_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("accounts.json")
}

pub fn load_accounts_in(data_dir: &Path) -> Option<AccountsState> {
    let path = accounts_path_in(data_dir);
    match load_json_file::<AccountsState>("accounts", &path) {
        LoadOutcome::Parsed(state) if state.schema_version <= SCHEMA_VERSION => Some(state),
        _ => None,
    }
}

pub fn save_accounts_in(data_dir: &Path, state: &AccountsState) -> std::io::Result<()> {
    let path = accounts_path_in(data_dir);
    save_json_atomic(data_dir, &path, state)
}

pub fn load_accounts() -> Option<AccountsState> {
    load_accounts_in(&default_data_dir())
}

pub fn save_accounts(state: &AccountsState) -> std::io::Result<()> {
    save_accounts_in(&default_data_dir(), state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AccountId, AgentProvider, ManagedAccount};

    #[test]
    fn save_then_load_in_temp_dir_round_trips() {
        let dir = std::env::temp_dir().join(format!("daruda-acct-test-{}", AccountId::new().0));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = AccountsState::default();
        state.accounts.push(ManagedAccount {
            id: AccountId::new(),
            provider: AgentProvider::Claude,
            email: None,
            organization: None,
            config_dir: dir.join("alice"),
            created_at: 0,
            last_authenticated_at: 0,
        });
        save_accounts_in(&dir, &state).unwrap();
        let back = load_accounts_in(&dir).unwrap();
        assert_eq!(back.accounts.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_rejects_newer_schema() {
        let dir = std::env::temp_dir().join(format!("daruda-acct-test-{}", AccountId::new().0));
        std::fs::create_dir_all(&dir).unwrap();
        let state = AccountsState {
            schema_version: SCHEMA_VERSION + 1,
            ..Default::default()
        };
        save_accounts_in(&dir, &state).unwrap();
        assert!(load_accounts_in(&dir).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
