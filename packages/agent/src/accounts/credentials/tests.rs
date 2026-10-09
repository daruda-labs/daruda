#[cfg(not(target_os = "macos"))]
use super::file::credentials_path_from;
use super::*;

#[test]
fn credentials_extract_token_and_plan_info() {
    let raw = r#"{
            "claudeAiOauth": {
                "accessToken": "sk-ant-oat01-abc",
                "refreshToken": "sk-ant-ort01-def",
                "expiresAt": 1778112000000,
                "scopes": ["user:inference", "user:profile"],
                "subscriptionType": "team",
                "rateLimitTier": "default_claude_ai_5x"
            }
        }"#;
    let (token, plan) = parse_credentials(raw).unwrap();
    assert_eq!(token, "sk-ant-oat01-abc");
    assert_eq!(plan.tier.as_deref(), Some("team"));
    assert_eq!(plan.qualifier.as_deref(), Some("default_claude_ai_5x"));
}

#[test]
fn credentials_tolerate_missing_plan_fields() {
    // Older payloads carry only the token — the read must still proceed.
    let (token, plan) =
        parse_credentials(r#"{ "claudeAiOauth": { "accessToken": "tok" } }"#).unwrap();
    assert_eq!(token, "tok");
    assert_eq!(plan.tier, None);
    assert_eq!(plan.qualifier, None);
}

#[test]
fn credentials_without_a_token_is_no_token() {
    let err =
        parse_credentials(r#"{ "claudeAiOauth": { "subscriptionType": "pro" } }"#).unwrap_err();
    assert!(matches!(err, FetchError::NoToken));
}

#[test]
fn malformed_json_is_no_token() {
    assert!(matches!(
        parse_credentials("{ not json").unwrap_err(),
        FetchError::NoToken
    ));
}

#[test]
fn credentials_accept_the_flat_shape() {
    // Claude Code CLI's Linux/Windows `.credentials.json` shape was never
    // confirmed against a live install — this fixture is the defensive
    // fallback (fields at the top level), not a verified-real sample.
    let raw = r#"{
            "accessToken": "sk-ant-oat01-flat",
            "subscriptionType": "pro",
            "rateLimitTier": "default_claude_ai"
        }"#;
    let (token, plan) = parse_credentials(raw).unwrap();
    assert_eq!(token, "sk-ant-oat01-flat");
    assert_eq!(plan.tier.as_deref(), Some("pro"));
    assert_eq!(plan.qualifier.as_deref(), Some("default_claude_ai"));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn credentials_path_prefers_the_claude_config_dir_override() {
    assert_eq!(
        credentials_path_from(
            Some(std::ffi::OsString::from("/custom/claude-dir")),
            Some(std::path::PathBuf::from("/home/someone")),
        ),
        Some(std::path::PathBuf::from(
            "/custom/claude-dir/.credentials.json"
        ))
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn credentials_path_falls_back_to_home_dot_claude() {
    assert_eq!(
        credentials_path_from(None, Some(std::path::PathBuf::from("/home/someone"))),
        Some(std::path::PathBuf::from(
            "/home/someone/.claude/.credentials.json"
        ))
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn credentials_path_is_none_without_config_dir_or_home() {
    assert_eq!(credentials_path_from(None, None), None);
}

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

/// The same entry must digest alike — otherwise the comparison this
/// exists for would report a clobber on every login.
#[test]
fn the_digest_is_stable_for_the_same_entry() {
    let entry = br#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-x"}}"#;
    assert_eq!(secret_digest(entry), secret_digest(entry));
    assert_ne!(secret_digest(entry), secret_digest(b"another entry"));
}

/// And it must never be the secret itself.
#[test]
fn the_digest_is_sha256_hex_not_the_secret() {
    assert_eq!(
        secret_digest(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let digest = secret_digest(br#"{"accessToken":"sk-ant-oat01-x"}"#);
    assert!(!digest.contains("sk-ant"));
}

/// Tests never read the user's own sign-in: the ambient store answers as
/// an empty one.
#[test]
fn the_ambient_store_reads_as_empty_under_test() {
    assert_eq!(system_credentials_digest(), None);
    assert!(read_system_credentials().is_err());
}
