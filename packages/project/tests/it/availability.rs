//! Public API tests for availability.

use daruda_project::lane::availability::{LaneAvailability, classify_dir};

#[test]
fn classify_present_missing_notadir() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(classify_dir(d.path()), LaneAvailability::Present);
    assert_eq!(
        classify_dir(&d.path().join("nope")),
        LaneAvailability::Missing
    );
    let file = d.path().join("f");
    std::fs::write(&file, b"x").unwrap();
    // A regular file is not a directory → read_dir fails → Missing.
    assert_eq!(classify_dir(&file), LaneAvailability::Missing);
}
