//! Public API tests for path identity.

use daruda_core::path::file_id;

#[test]
fn the_same_directory_keeps_its_identity() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(file_id(dir.path()).unwrap(), file_id(dir.path()).unwrap());
}

/// What the watcher relies on: a name reused for a new directory names
/// a different object, however soon after the old one moved away.
#[test]
fn a_replacement_under_the_same_name_is_a_different_object() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("watched");
    std::fs::create_dir(&root).unwrap();
    let before = file_id(&root).unwrap();
    std::fs::rename(&root, dir.path().join("old")).unwrap();
    std::fs::create_dir(&root).unwrap();
    assert_ne!(file_id(&root).unwrap(), before);
    assert_eq!(file_id(&dir.path().join("old")).unwrap(), before);
}

#[test]
fn a_missing_path_has_no_identity() {
    let dir = tempfile::tempdir().unwrap();
    assert!(file_id(&dir.path().join("absent")).is_err());
}
