//! Tests for the public `daruda_agent::skills::persist` API.

use daruda_agent::skills::persist::*;
#[test]
fn modal_safety_a_stale_edit_does_not_overwrite_an_external_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("SKILL.md");
    std::fs::write(&path, "external change").unwrap();
    assert!(update_skill_source(dir.path(), "old source", "draft").is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "external change");
}
