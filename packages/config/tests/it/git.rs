//! Public API tests for git.

use daruda_config::git::*;

#[test]
fn a_missing_section_keeps_every_default() {
    let parsed: GitConfig = toml::from_str("").unwrap();
    assert!(parsed.confirm_commit && parsed.confirm_push && parsed.default_commit_message);
}

#[test]
fn each_key_turns_off_on_its_own() {
    let parsed: GitConfig = toml::from_str("confirm_push = false").unwrap();
    assert!(parsed.confirm_commit);
    assert!(!parsed.confirm_push);
    assert!(parsed.default_commit_message);
}
