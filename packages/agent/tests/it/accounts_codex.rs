//! Tests for the public `daruda_agent::accounts::codex` API.

use daruda_agent::accounts::AccountRecipe;
use daruda_agent::accounts::codex::*;

#[test]
fn cleanup_removes_the_account_dir_but_not_the_symlink_targets() {
    let source = tempfile::tempdir().expect("source");
    std::fs::create_dir_all(source.path().join("skills")).expect("skills");
    std::fs::write(source.path().join("skills").join("a.md"), b"skill").expect("skill file");
    let dir = tempfile::tempdir().expect("dest").keep();
    daruda_core::path::symlink(source.path().join("skills"), dir.join("skills")).expect("symlink");

    CodexRecipe.cleanup(&dir);

    assert!(!dir.exists());
    assert_eq!(
        std::fs::read(source.path().join("skills").join("a.md")).expect("source survives"),
        b"skill"
    );
}

#[test]
fn cleanup_is_a_noop_when_dir_is_already_gone() {
    let dir = std::env::temp_dir().join("daruda-codex-recipe-missing-xyz");
    CodexRecipe.cleanup(&dir);
}
