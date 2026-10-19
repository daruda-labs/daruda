//! A log a newer machine schema wrote is refused as too new, whatever part
//! of it this build would fail to read, and never repaired or dropped.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use daruda_runlog::{FileBackend, Machine as _, OpenError, RunLog, Versioned};

use crate::toy::{BUILD, NewerToy, Toy, add};

fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

fn is_newer_schema(opened: &Result<impl Sized, OpenError>) -> bool {
    matches!(
        opened,
        Err(OpenError::TooNew {
            what: Versioned::Schema,
            found,
            supported,
        }) if *found == NewerToy::SCHEMA && *supported == Toy::SCHEMA
    )
}

#[test]
fn an_event_from_a_newer_schema_is_too_new_not_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = RunLog::<NewerToy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    log.commit(add(2)).unwrap();
    let next = log.last_seq() + 1;
    drop(log);
    // A variant the older build has never heard of.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.path().join("log.0.jsonl"))
        .unwrap();
    writeln!(
        file,
        "{{\"v\":1,\"seq\":{next},\"at_ms\":0,\"kind\":\"event\",\"event\":{{\"type\":\"teleport\"}}}}"
    )
    .unwrap();
    drop(file);
    let before = files(dir.path());

    assert!(is_newer_schema(&RunLog::<Toy, _>::open(
        FileBackend::new(dir.path()),
        BUILD
    )));
    assert!(is_newer_schema(&RunLog::<Toy, _>::read_history(
        &FileBackend::new(dir.path())
    )));
    assert_eq!(files(dir.path()), before);
}

#[test]
fn an_older_schema_is_read_by_a_newer_build() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    log.commit(add(2)).unwrap();
    drop(log);

    let log = RunLog::<NewerToy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    assert_eq!(log.state().total, 2);
}

/// A newer build compacted; the older generation is still on disk, as a
/// lost drop leaves it. Falling back to it would drop the newer one.
#[test]
fn a_newer_schema_snapshot_is_refused_not_fallen_back_past() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = RunLog::<NewerToy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    log.commit(add(7)).unwrap();
    let log0 = std::fs::read(dir.path().join("log.0.jsonl")).unwrap();
    log.compact().unwrap();
    log.commit(add(1)).unwrap();
    drop(log);
    std::fs::write(dir.path().join("log.0.jsonl"), log0).unwrap();
    let before = files(dir.path());

    assert!(is_newer_schema(&RunLog::<Toy, _>::open(
        FileBackend::new(dir.path()),
        BUILD
    )));
    assert_eq!(files(dir.path()), before);
}

/// A newer generation whose snapshot does not read, but whose log a newer
/// build wrote: this build cannot tell it is broken rather than newer, so it
/// must not drop it.
#[test]
fn a_newer_generation_log_is_not_dropped_behind_a_broken_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    log.commit(add(7)).unwrap();
    drop(log);
    let mut newer = RunLog::<NewerToy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    newer.compact().unwrap();
    drop(newer);
    let mut newer = RunLog::<NewerToy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    newer.commit(add(1)).unwrap();
    drop(newer);
    std::fs::write(dir.path().join("log.0.jsonl"), b"").unwrap();
    std::fs::write(dir.path().join("snapshot.1.json"), b"{\"torn").unwrap();
    let before = files(dir.path());

    assert!(is_newer_schema(&RunLog::<Toy, _>::open(
        FileBackend::new(dir.path()),
        BUILD
    )));
    assert_eq!(files(dir.path()), before);
}
