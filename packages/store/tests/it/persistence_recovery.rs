//! Recoverable JSON records.

use daruda_store::persistence::*;
use serde::Serialize;

#[derive(Debug, PartialEq, Serialize, serde::Deserialize)]
struct State {
    value: u32,
}

#[test]
fn schema_probes_recover_without_discarding_unknown_fields() {
    #[derive(serde::Deserialize)]
    struct Probe {
        schema_version: u32,
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let original = serde_json::json!({"schema_version": 2, "unknown": ["preserve", "all", "rows"]});
    daruda_store::persistence::save_json_atomic(dir.path(), &path, &original).unwrap();
    daruda_store::persistence::save_json_atomic(
        dir.path(),
        &path,
        &serde_json::json!({"schema_version": 2, "unknown": []}),
    )
    .unwrap();
    std::fs::write(&path, "corrupt").unwrap();
    assert_eq!(
        load_recoverable_json::<Probe>("probe", &path)
            .unwrap()
            .unwrap()
            .schema_version,
        2
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(),
        original
    );
    assert!(
        daruda_store::persistence::save_json_atomic(
            dir.path(),
            &path,
            &serde_json::json!({"schema_version": 1})
        )
        .is_err()
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(),
        original
    );
}

#[test]
fn damaged_source_is_preserved_and_last_good_generation_is_restored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    save_recoverable_json(dir.path(), &path, &State { value: 1 }).unwrap();
    save_recoverable_json(dir.path(), &path, &State { value: 2 }).unwrap();
    std::fs::write(&path, b"damaged").unwrap();
    assert_eq!(
        load_recoverable_json::<State>("test", &path).unwrap(),
        Some(State { value: 1 })
    );
    let preserved = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("corrupt-state-")
        })
        .unwrap();
    assert_eq!(std::fs::read(preserved.path()).unwrap(), b"damaged");
}

#[test]
fn corrupt_records_cannot_be_overwritten_or_mistaken_for_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    assert!(
        load_recoverable_json::<State>("test", &path)
            .unwrap()
            .is_none()
    );
    std::fs::write(&path, b"damaged").unwrap();
    assert!(load_recoverable_json::<State>("test", &path).is_err());
    assert!(save_recoverable_json(dir.path(), &path, &State { value: 1 }).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"damaged");
}

#[test]
fn intentional_deletion_does_not_restore_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    save_recoverable_json(dir.path(), &path, &State { value: 1 }).unwrap();
    save_recoverable_json(dir.path(), &path, &State { value: 2 }).unwrap();
    delete_recoverable_json(&path).unwrap();
    assert!(
        load_recoverable_json::<State>("test", &path)
            .unwrap()
            .is_none()
    );
}
