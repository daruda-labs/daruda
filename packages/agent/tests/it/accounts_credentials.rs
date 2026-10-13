//! Public API tests for accounts credentials.

use daruda_agent::accounts::credentials::*;

/// A config dir whose credentials landed in the file rather than the
/// Keychain still has credentials. Reading only the Keychain reports a
/// successful login as a failed one — and the add flow deletes the
/// directory on that answer.
#[test]
fn a_config_dir_with_only_a_credentials_file_is_still_signed_in() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join(".credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-x","subscriptionType":"pro"}}"#,
    )
    .expect("fixture");
    let (token, plan) = read_scoped_credentials(dir.path()).expect("the file is read");
    assert_eq!(token, "sk-ant-oat01-x");
    assert_eq!(plan.tier.as_deref(), Some("pro"));
}

/// Neither store holding anything is still "signed out".
#[test]
fn an_empty_config_dir_has_no_credentials() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(read_scoped_credentials(dir.path()).is_err());
}
