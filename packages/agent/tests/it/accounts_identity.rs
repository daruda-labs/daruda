//! Tests for the public `daruda_agent::accounts::identity` API.

use daruda_agent::accounts::identity::*;
#[test]
fn reads_email_and_org_from_oauth_account() {
    let dir = std::env::temp_dir().join(format!("daruda-id-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"alice@acme.com","organizationName":"Acme"}}"#,
    )
    .unwrap();
    let id = read_account_identity(&dir);
    assert_eq!(id.email.as_deref(), Some("alice@acme.com"));
    assert_eq!(id.organization.as_deref(), Some("Acme"));
    std::fs::remove_dir_all(&dir).ok();
}
/// The block has been written under a second file name too; reading only
/// the first leaves the account with no identity at all.
#[test]
fn reads_the_oauth_account_from_the_alternate_config_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join(".config.json"),
        r#"{"oauthAccount":{"emailAddress":"b@x.com","organizationName":"Org"}}"#,
    )
    .unwrap();
    let id = read_account_identity(dir.path());
    assert_eq!(id.email.as_deref(), Some("b@x.com"));
}

/// `.claude.json` wins when both carry a block.
#[test]
fn the_primary_config_file_wins() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"primary@x.com"}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join(".config.json"),
        r#"{"oauthAccount":{"emailAddress":"alternate@x.com"}}"#,
    )
    .unwrap();
    assert_eq!(
        read_account_identity(dir.path()).email.as_deref(),
        Some("primary@x.com")
    );
}

/// A file present but carrying no block must not stop the search.
#[test]
fn a_blockless_config_file_falls_through_to_the_next() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(".claude.json"), r#"{"other":1}"#).unwrap();
    std::fs::write(
        dir.path().join(".config.json"),
        r#"{"oauthAccount":{"email":"fallback@x.com"}}"#,
    )
    .unwrap();
    assert_eq!(
        read_account_identity(dir.path()).email.as_deref(),
        Some("fallback@x.com"),
        "the alternate file and the alternate field name both have to be tried"
    );
}

/// An empty string is not an identity — it renders as a blank row.
#[test]
fn a_blank_email_is_not_captured() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"  ","email":"real@x.com"}}"#,
    )
    .unwrap();
    assert_eq!(
        read_account_identity(dir.path()).email.as_deref(),
        Some("real@x.com")
    );
}

#[test]
fn missing_file_yields_empty_identity() {
    let id = read_account_identity(std::path::Path::new("/nonexistent-xyz"));
    assert!(id.email.is_none() && id.organization.is_none());
}
