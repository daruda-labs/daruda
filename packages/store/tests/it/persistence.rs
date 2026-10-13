//! Shared JSON load / atomic save helpers.

use daruda_store::persistence::*;

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
struct Sample {
    x: i32,
}

#[test]
fn missing_file_returns_missing_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nope.json");
    let result: LoadOutcome<Sample> = load_json_file("test", &path);
    assert!(matches!(result, LoadOutcome::Missing));
}

#[test]
fn corrupt_json_returns_corrupt_outcome_not_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.json");
    std::fs::write(&path, b"{ not valid json").unwrap();
    let result: LoadOutcome<Sample> = load_json_file("test", &path);
    assert!(
        matches!(result, LoadOutcome::Corrupt),
        "expected Corrupt, got something else"
    );
}

#[test]
fn save_then_load_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("good.json");
    save_json_atomic(dir.path(), &path, &Sample { x: 42 }).unwrap();
    let result: LoadOutcome<Sample> = load_json_file("test", &path);
    match result {
        LoadOutcome::Parsed(s) => assert_eq!(s, Sample { x: 42 }),
        _ => panic!("expected Parsed"),
    }
}

#[test]
fn load_outcome_into_option_collapses_missing_and_corrupt() {
    let m: LoadOutcome<Sample> = LoadOutcome::Missing;
    assert!(m.into_option().is_none());
    let c: LoadOutcome<Sample> = LoadOutcome::Corrupt;
    assert!(c.into_option().is_none());
    let p: LoadOutcome<Sample> = LoadOutcome::Parsed(Sample { x: 7 });
    assert_eq!(p.into_option(), Some(Sample { x: 7 }));
}
