//! Public API tests for account login.

use daruda_agent::accounts::login::{LoginProcessHandle, is_oauth_denied};

#[test]
fn detects_oauth_denied_marker() {
    assert!(is_oauth_denied("... error: access_denied ..."));
    assert!(is_oauth_denied("Authorization was denied by the user."));
    assert!(!is_oauth_denied("Login successful"));
}

#[test]
fn denial_check_is_case_insensitive() {
    assert!(is_oauth_denied("ACCESS_DENIED"));
    assert!(is_oauth_denied("Sign-In Denied"));
}

/// Compile+shape test for Finding 1: `LoginProcessHandle` must be
/// cloneable so `Workspace` can stash one clone (e.g.
/// `PendingLogin::InProgress { handle }`) while another clone (or the
/// original) is used elsewhere. No process is spawned here.
#[test]
fn login_process_handle_is_clone() {
    fn assert_clone<T: Clone>() {}
    assert_clone::<LoginProcessHandle>();
}
