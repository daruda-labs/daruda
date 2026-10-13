//! Account model serialization.

use daruda_store::accounts::*;

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
    let names: Vec<&str> = AccountRecipeId::all()
        .map(|id| match id {
            AccountRecipeId::Claude => "claude",
            AccountRecipeId::Codex => "codex",
        })
        .collect();
    assert_eq!(names, ["claude", "codex"]);
    assert_eq!(AccountRecipeId::count(), names.len());
}
