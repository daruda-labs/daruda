//! `accounts.json` load/save via the shared JSON helpers.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use fs4::fs_std::FileExt as _;

use crate::accounts::{AccountsState, SCHEMA_VERSION};
use crate::observability::error_report::{ErrorReport, ErrorSeverity};
use crate::observability::log_writer::LogWriter;
use crate::persistence::{LoadOutcome, default_data_dir, load_json_file, save_json_atomic};

pub fn accounts_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("accounts.json")
}

fn accounts_lock_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("accounts.json.lock")
}

/// Load `accounts.json`, upgrading an older schema in place. `None` for a
/// missing, unreadable, or newer-than-this-build file.
pub fn load_accounts_in(data_dir: &Path) -> Option<AccountsState> {
    let path = accounts_path_in(data_dir);
    let state = read_accounts_file(&path)?;
    if state.schema_version == SCHEMA_VERSION {
        return Some(state);
    }
    migrate_to_current(data_dir, &path, state)
}

/// Parse the file, rejecting one written by a newer daruda.
fn read_accounts_file(path: &Path) -> Option<AccountsState> {
    match load_json_file::<AccountsState>("accounts", path) {
        LoadOutcome::Parsed(state) if state.schema_version <= SCHEMA_VERSION => Some(state),
        _ => None,
    }
}

/// Stamp `state` at the current schema and write it back.
///
/// v1's `default_by_provider` is dropped rather than carried over: on disk an
/// explicit "Set default" and a first account auto-promoted by the login flow
/// were the same field, so a preserved value is more likely the bug than the
/// user's intent. Starting empty means new panes are System until one click.
fn migrate_to_current(
    data_dir: &Path,
    path: &Path,
    mut state: AccountsState,
) -> Option<AccountsState> {
    state.schema_version = SCHEMA_VERSION;
    // Another window may have written the file between our read and this
    // write; take its result rather than overwriting it with ours.
    match load_json_file::<AccountsState>("accounts", path) {
        LoadOutcome::Parsed(disk) if disk.schema_version > SCHEMA_VERSION => return None,
        LoadOutcome::Parsed(disk) if disk.schema_version == SCHEMA_VERSION => return Some(disk),
        _ => {}
    }
    if let Err(e) = save_accounts_in(data_dir, &state) {
        // The in-memory state is still correct, so the caller gets it either
        // way; the next successful save re-attempts the upgrade.
        LogWriter::log(
            ErrorReport::new("Failed to save accounts.json after a schema migration")
                .severity(ErrorSeverity::Warning)
                .from_error(&e)
                .at(file!(), line!())
                .dedup("store.accounts.migrate_save")
                .build(),
        );
    }
    Some(state)
}

pub fn save_accounts_in(data_dir: &Path, state: &AccountsState) -> std::io::Result<()> {
    let path = accounts_path_in(data_dir);
    save_json_atomic(data_dir, &path, state)
}

/// Load, mutate, and save `accounts.json` while holding a sibling lock file.
///
/// The plain load/save helpers remain available for callers that only read or
/// already hold stronger coordination. Account UI flows that merge user edits
/// into disk state should use this to avoid the read-modify-write race between
/// open windows.
pub fn mutate_accounts_in<R>(
    data_dir: &Path,
    mutate: impl FnOnce(&mut AccountsState) -> R,
) -> std::io::Result<(AccountsState, R)> {
    std::fs::create_dir_all(data_dir)?;
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(accounts_lock_path_in(data_dir))?;
    lock_file.lock_exclusive()?;

    let mut state = load_accounts_in(data_dir).unwrap_or_default();
    let result = mutate(&mut state);
    save_accounts_in(data_dir, &state)?;
    Ok((state, result))
}

pub fn load_accounts() -> Option<AccountsState> {
    load_accounts_in(&default_data_dir())
}

pub fn save_accounts(state: &AccountsState) -> std::io::Result<()> {
    save_accounts_in(&default_data_dir(), state)
}

pub fn mutate_accounts<R>(
    mutate: impl FnOnce(&mut AccountsState) -> R,
) -> std::io::Result<(AccountsState, R)> {
    mutate_accounts_in(&default_data_dir(), mutate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AccountId, AccountRecipeId, ManagedAccount};

    #[test]
    fn save_then_load_in_temp_dir_round_trips() {
        let dir = std::env::temp_dir().join(format!("daruda-acct-test-{}", AccountId::new().0));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = AccountsState::default();
        state.accounts.push(ManagedAccount {
            id: AccountId::new(),
            recipe: AccountRecipeId::Claude,
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
    fn mutate_accounts_in_loads_mutates_and_saves() {
        let dir = std::env::temp_dir().join(format!("daruda-acct-test-{}", AccountId::new().0));
        std::fs::create_dir_all(&dir).unwrap();
        let account_id = AccountId::new();

        let (state, added) = mutate_accounts_in(&dir, |state| {
            state.accounts.push(ManagedAccount {
                id: account_id,
                recipe: AccountRecipeId::Claude,
                email: Some("alice@company.com".into()),
                organization: None,
                config_dir: dir.join("alice"),
                created_at: 1,
                last_authenticated_at: 2,
            });
            true
        })
        .expect("mutate accounts");

        assert!(added);
        assert_eq!(state.accounts.len(), 1);
        let back = load_accounts_in(&dir).expect("load mutated state");
        assert_eq!(back.accounts[0].id, account_id);
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

    /// Hand-written v1 `accounts.json`: two account records (one Codex) and a
    /// populated `default_by_provider`.
    fn write_v1_file(dir: &Path, claude_id: AccountId, codex_id: AccountId) {
        let json = format!(
            r#"{{
  "accounts": [
    {{
      "id": "{claude_id}",
      "provider": "claude",
      "email": "alice@company.com",
      "organization": "Acme",
      "config_dir": "/data/accounts/{claude_id}",
      "created_at": 11,
      "last_authenticated_at": 22
    }},
    {{
      "id": "{codex_id}",
      "provider": "codex",
      "config_dir": "/data/accounts/{codex_id}",
      "created_at": 33,
      "last_authenticated_at": 44
    }}
  ],
  "default_by_provider": {{ "claude": "{claude_id}" }},
  "schema_version": 1
}}"#,
            claude_id = claude_id.0,
            codex_id = codex_id.0
        );
        std::fs::write(accounts_path_in(dir), json).unwrap();
    }

    #[test]
    fn v1_file_keeps_its_accounts_and_drops_its_defaults() {
        let dir = std::env::temp_dir().join(format!("daruda-acct-test-{}", AccountId::new().0));
        std::fs::create_dir_all(&dir).unwrap();
        let claude_id = AccountId::new();
        let codex_id = AccountId::new();
        write_v1_file(&dir, claude_id, codex_id);

        let loaded = load_accounts_in(&dir).expect("a v1 file still loads");
        assert_eq!(
            loaded.accounts,
            vec![
                ManagedAccount {
                    id: claude_id,
                    recipe: AccountRecipeId::Claude,
                    email: Some("alice@company.com".into()),
                    organization: Some("Acme".into()),
                    config_dir: PathBuf::from(format!("/data/accounts/{}", claude_id.0)),
                    created_at: 11,
                    last_authenticated_at: 22,
                },
                ManagedAccount {
                    id: codex_id,
                    recipe: AccountRecipeId::Codex,
                    email: None,
                    organization: None,
                    config_dir: PathBuf::from(format!("/data/accounts/{}", codex_id.0)),
                    created_at: 33,
                    last_authenticated_at: 44,
                },
            ],
            "account records must survive the migration untouched"
        );
        assert!(
            loaded.default_by_recipe.is_empty(),
            "the v1 default map is discarded"
        );
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);

        let on_disk = std::fs::read_to_string(accounts_path_in(&dir)).unwrap();
        assert!(on_disk.contains("\"schema_version\": 2"), "{on_disk}");
        assert!(!on_disk.contains("default_by_provider"), "{on_disk}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn v2_file_round_trips_its_default() {
        let dir = std::env::temp_dir().join(format!("daruda-acct-test-{}", AccountId::new().0));
        std::fs::create_dir_all(&dir).unwrap();
        let id = AccountId::new();
        let mut state = AccountsState::default();
        state.accounts.push(ManagedAccount {
            id,
            recipe: AccountRecipeId::Claude,
            email: None,
            organization: None,
            config_dir: dir.join("alice"),
            created_at: 0,
            last_authenticated_at: 0,
        });
        state.default_by_recipe.insert(AccountRecipeId::Claude, id);
        save_accounts_in(&dir, &state).unwrap();

        let back = load_accounts_in(&dir).expect("a v2 file loads");
        assert_eq!(
            back.default_by_recipe.get(&AccountRecipeId::Claude),
            Some(&id),
            "a real default must not be wiped by a second migration pass"
        );
        assert_eq!(back.schema_version, SCHEMA_VERSION);
        std::fs::remove_dir_all(&dir).ok();
    }
}
