use std::path::Path;

use daruda_runlog::{Body, CatchUp, FORMAT_VERSION, FileBackend, OpenError, RunLog};

use crate::toy::{BUILD, Toy, add, ticket};

fn log_path(dir: &Path) -> std::path::PathBuf {
    dir.join("log.0.jsonl")
}

/// A generation-0 log holding an `opened` record and two events.
fn seeded(dir: &Path) -> Vec<u8> {
    let mut log = RunLog::<Toy, _>::open(FileBackend::new(dir), BUILD).unwrap();
    log.commit(add(2)).unwrap();
    log.commit(ticket()).unwrap();
    drop(log);
    std::fs::read(log_path(dir)).unwrap()
}

fn lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_newer_record_version_is_refused_and_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let mut bytes = seeded(dir.path());
    let newer = format!(
        "{{\"v\":{},\"seq\":4,\"at_ms\":0,\"kind\":\"wholly_new\"}}\n",
        FORMAT_VERSION + 1
    );
    bytes.extend_from_slice(newer.as_bytes());
    // A torn tail must not be cut either: refusing means writing nothing.
    bytes.extend_from_slice(b"{\"v\":1,");
    std::fs::write(log_path(dir.path()), &bytes).unwrap();

    let err = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD)
        .err()
        .unwrap();
    assert!(matches!(
        err,
        OpenError::TooNew { found, supported }
            if found == FORMAT_VERSION + 1 && supported == FORMAT_VERSION
    ));
    assert_eq!(std::fs::read(log_path(dir.path())).unwrap(), bytes);
}

#[test]
fn a_complete_newer_record_without_its_newline_is_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let mut bytes = seeded(dir.path());
    let newer = format!(
        "{{\"v\":{},\"seq\":4,\"at_ms\":0,\"kind\":\"wholly_new\"}}",
        FORMAT_VERSION + 1
    );
    bytes.extend_from_slice(newer.as_bytes());
    std::fs::write(log_path(dir.path()), &bytes).unwrap();

    let opened = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD);
    assert!(matches!(
        opened,
        Err(OpenError::TooNew { found, supported })
            if found == FORMAT_VERSION + 1 && supported == FORMAT_VERSION
    ));
    assert_eq!(std::fs::read(log_path(dir.path())).unwrap(), bytes);
}

#[test]
fn a_corrupt_middle_line_is_refused_with_its_line_number() {
    let dir = tempfile::tempdir().unwrap();
    let mut rows = lines(&seeded(dir.path()));
    rows[1] = "{not json".into();
    std::fs::write(log_path(dir.path()), rows.join("\n") + "\n").unwrap();

    let err = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD)
        .err()
        .unwrap();
    assert!(matches!(err, OpenError::Corrupt { line: 2 }), "{err}");
}

#[test]
fn an_unknown_event_payload_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut rows = lines(&seeded(dir.path()));
    let last = rows.len() - 1;
    rows[last] = rows[last].replace("\"ticket\"", "\"teleport\"");
    std::fs::write(log_path(dir.path()), rows.join("\n") + "\n").unwrap();

    let err = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD)
        .err()
        .unwrap();
    assert!(matches!(err, OpenError::Corrupt { line } if line == last as u64 + 1));
}

#[test]
fn a_torn_last_line_is_cut_and_appending_continues() {
    let dir = tempfile::tempdir().unwrap();
    let mut bytes = seeded(dir.path());
    let whole = bytes.len();
    bytes.extend_from_slice(b"{\"v\":1,\"seq\":4,\"at");
    std::fs::write(log_path(dir.path()), &bytes).unwrap();

    let mut log = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    assert_eq!(log.state().total, 2);
    log.commit(add(1)).unwrap();
    drop(log);

    let after = std::fs::read(log_path(dir.path())).unwrap();
    assert_eq!(&after[..whole], &bytes[..whole]);
    assert_eq!(after.last(), Some(&b'\n'));
    let rows = lines(&after);
    assert!(rows.iter().all(|row| row.starts_with("{\"v\":1,")));
    let reopened = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    assert_eq!(reopened.state().total, 3);
}

#[test]
fn a_whole_last_line_without_its_newline_is_cut() {
    let dir = tempfile::tempdir().unwrap();
    let mut bytes = seeded(dir.path());
    bytes.pop();
    std::fs::write(log_path(dir.path()), &bytes).unwrap();

    let log = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    // The ticket line lost its newline, so it was never acknowledged.
    assert!(log.state().tickets.is_empty());
    assert_eq!(log.state().total, 2);
}

#[test]
fn every_open_appends_an_opened_record_with_a_higher_epoch() {
    let dir = tempfile::tempdir().unwrap();
    let mut epochs = Vec::new();
    for round in 0..3 {
        let mut log = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
        let CatchUp::Records(records) = log.since(log.last_seq() - 1).unwrap() else {
            panic!("the record just written is in the current generation");
        };
        let [record] = records.as_slice() else {
            panic!("expected only the opened record, got {}", records.len());
        };
        let Body::Opened { epoch, build } = &record.body else {
            panic!("the last record after open is `opened`");
        };
        assert_eq!(build, BUILD);
        epochs.push(*epoch);
        if round == 1 {
            // Compaction must carry the epoch into the next generation.
            log.compact().unwrap();
        }
    }
    assert_eq!(epochs, vec![1, 2, 3]);
}
