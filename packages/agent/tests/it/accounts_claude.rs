//! Public API tests for accounts claude.

use daruda_agent::accounts::{AccountRecipe, ClaudeRecipe};

#[test]
fn cleanup_removes_the_config_dir() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    std::fs::write(dir.join("marker"), b"x").unwrap();
    ClaudeRecipe.cleanup(&dir);
    assert!(!dir.exists());
}

#[test]
fn cleanup_is_a_noop_when_dir_is_already_gone() {
    let dir = std::env::temp_dir().join("daruda-agent-recipe-missing-xyz");
    // Never created — must not panic on a missing dir.
    ClaudeRecipe.cleanup(&dir);
}

#[test]
fn has_credentials_is_false_for_a_dir_with_no_keychain_item() {
    let dir = std::env::temp_dir().join(format!("daruda-recipe-creds-{}", std::process::id()));
    assert!(!ClaudeRecipe.has_credentials(&dir));
}

#[test]
fn read_identity_parses_the_oauth_account_block() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"a@x.com","organizationName":"Org"}}"#,
    )
    .unwrap();
    let identity = ClaudeRecipe.read_identity(dir.path());
    assert_eq!(identity.email.as_deref(), Some("a@x.com"));
    assert_eq!(identity.organization.as_deref(), Some("Org"));
}
