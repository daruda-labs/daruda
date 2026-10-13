//! Public API tests for account env.

use daruda_config::account_env::*;
use std::path::Path;

/// The ambient environment is the *absence* of both overrides, not an
/// empty injection: a pane with no managed account inherits the user's
/// own auth vars, and a login meant for that pane has to as well, or it
/// would sign into a place the pane never reads.
#[test]
fn the_ambient_environment_overrides_nothing() {
    let e = AccountEnv::ambient();
    assert!(e.inject.is_empty());
    assert!(e.strip.is_empty());
}

#[test]
fn injects_config_dir_and_strips_auth_overrides() {
    let strip: &[&str] = &["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"];
    let e = account_env(
        "CLAUDE_CONFIG_DIR",
        Path::new("/data/claude-accounts/alice"),
        strip,
    );
    assert!(
        e.inject
            .iter()
            .any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == "/data/claude-accounts/alice")
    );
    assert!(e.strip.contains(&"ANTHROPIC_API_KEY"));
    assert!(e.strip.contains(&"CLAUDE_CODE_OAUTH_TOKEN"));
}

#[test]
fn strip_list_is_passed_through_verbatim() {
    let e = account_env("SOME_ENV", Path::new("/tmp/x"), &[]);
    assert!(e.strip.is_empty());
}
