//! System-info summary and home-path redaction.

use daruda_store::observability::system_info::*;
use std::path::PathBuf;

#[test]
fn summary_contains_os_and_arch() {
    // We cannot reliably test `set_app_version` here because the
    // OnceLock is process-global and other tests in this module
    // may have set it. Just assert the static parts are present.
    let s = summary();
    assert!(s.starts_with("daruda "));
    assert!(s.contains(std::env::consts::OS));
    assert!(s.contains(std::env::consts::ARCH));
}

#[test]
fn summary_omits_privacy_sensitive_fields() {
    let s = summary();
    // No environment variables.
    if let Ok(home) = std::env::var("HOME") {
        assert!(!s.contains(&home), "summary leaked $HOME");
    }
    if let Ok(user) = std::env::var("USER") {
        // Allow short usernames that might collide with "macos" etc.
        // — assert only when the username is at least 4 chars.
        if user.len() >= 4 {
            assert!(!s.contains(&user), "summary leaked $USER");
        }
    }
}

#[test]
fn redact_home_replaces_prefix() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let nested = home.join("git").join("daruda");
    let redacted = redact_home(&nested);
    assert!(
        redacted.starts_with("~"),
        "expected leading ~, got {redacted:?}"
    );
    assert!(
        !redacted.contains(&home.to_string_lossy().into_owned()),
        "redact_home leaked the literal home prefix"
    );
}

/// The home directory reached another way — here through a symlink —
/// is still the user's name, and still hidden.
#[cfg(unix)]
#[test]
fn redact_home_hides_a_symlinked_spelling_of_home() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("home-link");
    if daruda_core::path::symlink(&home, &link).is_err() {
        return;
    }
    let redacted = redact_home(&link);
    assert_eq!(redacted, "~");
}

#[test]
fn redact_home_returns_path_unchanged_when_outside_home() {
    let p = PathBuf::from("/tmp/some/path");
    let r = redact_home(&p);
    assert_eq!(r, "/tmp/some/path");
}

#[test]
fn redact_home_handles_exact_home() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    assert_eq!(redact_home(&home), "~");
}
