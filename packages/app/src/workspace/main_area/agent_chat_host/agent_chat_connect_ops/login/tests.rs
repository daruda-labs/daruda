use super::login_revives_pane;
use daruda_acp::Remedy;
use daruda_agent::accounts::LoginTarget;
use daruda_store::accounts::AccountRecipeId;
fn claude_system() -> LoginTarget {
    LoginTarget::System {
        recipe: AccountRecipeId::Claude,
    }
}

#[test]
fn a_pane_blocked_on_this_login_is_revived() {
    assert!(login_revives_pane(
        claude_system(),
        Some(claude_system()),
        Some(Remedy::Reauthenticate)
    ));
}

/// The signed-in credentials are not this pane's, so reconnecting it would
/// re-run the same handshake with the same expired login.
#[test]
fn a_pane_on_other_credentials_is_left_alone() {
    assert!(!login_revives_pane(
        claude_system(),
        Some(LoginTarget::System {
            recipe: AccountRecipeId::Codex
        }),
        Some(Remedy::Reauthenticate)
    ));
}

/// An organization-blocked account is the case that must not reconnect: the
/// login succeeds and changes nothing, so an automatic retry would fail
/// identically while looking like the app is making progress.
#[test]
fn a_failure_a_login_cannot_fix_is_not_retried() {
    for remedy in [
        Remedy::ExternalAction,
        Remedy::Configure,
        Remedy::Retry,
        Remedy::NoneAvailable,
    ] {
        assert!(
            !login_revives_pane(claude_system(), Some(claude_system()), Some(remedy)),
            "{remedy:?} is not fixed by signing in"
        );
    }
}

/// A pane that never failed is mid-conversation. A login elsewhere in the
/// app must not tear its live session down.
#[test]
fn a_working_pane_is_never_reconnected() {
    assert!(!login_revives_pane(
        claude_system(),
        Some(claude_system()),
        None
    ));
}

#[test]
fn a_pane_with_no_resolvable_login_is_left_alone() {
    assert!(!login_revives_pane(
        claude_system(),
        None,
        Some(Remedy::Reauthenticate)
    ));
}
