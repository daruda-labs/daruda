//! `accounts.json` persistence.

use std::path::{Path, PathBuf};

use daruda_store::accounts::persistence::*;
use daruda_store::accounts::{AccountId, AccountRecipeId, ManagedAccount};
use daruda_store::accounts::{AccountsState, SCHEMA_VERSION};

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

/// A write must not treat a file it cannot read as an empty list — saving
/// the result would erase every account in it. The mutation never runs,
/// and the bytes on disk are exactly what they were.
#[test]
fn mutate_leaves_a_file_it_cannot_read_untouched() {
    let newer = serde_json::to_string(&AccountsState {
        schema_version: SCHEMA_VERSION + 1,
        ..Default::default()
    })
    .unwrap();
    for (label, bytes) in [("corrupt", "{ not json".to_owned()), ("newer", newer)] {
        let dir = std::env::temp_dir().join(format!("daruda-acct-test-{}", AccountId::new().0));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(accounts_path_in(&dir), &bytes).unwrap();

        let mut ran = false;
        let outcome = mutate_accounts_in(&dir, |_| ran = true);

        assert!(outcome.is_err(), "{label}: the write is refused");
        assert!(!ran, "{label}: the mutation never sees an empty list");
        assert_eq!(
            std::fs::read_to_string(accounts_path_in(&dir)).unwrap(),
            bytes,
            "{label}: the file is left as it was"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
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
