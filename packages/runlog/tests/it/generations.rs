use daruda_runlog::testing::{DirOp, FaultBackend};
use daruda_runlog::{CatchUp, FileBackend, LogBackend, RunLog};

use crate::toy::{BUILD, Toy, ToyState, add, insert, ticket};

fn compacted_log(lost_from: Option<DirOp>) -> (RunLog<Toy, FaultBackend>, ToyState) {
    let mut backend = FaultBackend::new();
    if let Some(op) = lost_from {
        backend = backend.lose_dir_ops_from(op);
    }
    let mut log = RunLog::<Toy, _>::open(backend, BUILD).unwrap();
    log.commit(add(3)).unwrap();
    log.commit(insert("a")).unwrap();
    let before = log.state().clone();
    log.compact().unwrap();
    log.commit(ticket()).unwrap();
    (log, before)
}

#[test]
fn a_lost_snapshot_rename_reopens_the_old_generation() {
    let (log, before) = compacted_log(Some(DirOp::ReplaceSnapshot(1)));
    let reopened = RunLog::<Toy, _>::open(log.backend().crash(), BUILD).unwrap();
    assert_eq!(reopened.state(), &before);
}

#[test]
fn a_lost_drop_reopens_the_new_generation_and_removes_the_old() {
    let (log, _) = compacted_log(Some(DirOp::DropGeneration(0)));
    let disk = log.backend().crash();
    assert_eq!(disk.generations().unwrap(), vec![0, 1]);
    let reopened = RunLog::<Toy, _>::open(disk, BUILD).unwrap();
    assert_eq!(reopened.state(), log.state());
    assert_eq!(reopened.backend().generations().unwrap(), vec![1]);
}

#[test]
fn a_lost_log_creation_reopens_the_snapshot_with_an_empty_log() {
    let (log, before) = compacted_log(Some(DirOp::CreateLog(1)));
    let disk = log.backend().crash();
    assert!(disk.read(1).unwrap().log.is_empty());
    let reopened = RunLog::<Toy, _>::open(disk, BUILD).unwrap();
    assert_eq!(reopened.state(), &before);
}

#[test]
fn a_snapshot_with_a_wrong_checksum_falls_back_to_the_lower_generation() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    log.commit(add(7)).unwrap();
    // Keep generation 0 on disk, as a lost drop would.
    let log0 = std::fs::read(dir.path().join("log.0.jsonl")).unwrap();
    log.compact().unwrap();
    log.commit(add(1)).unwrap();
    drop(log);
    std::fs::write(dir.path().join("log.0.jsonl"), log0).unwrap();

    let snapshot = dir.path().join("snapshot.1.json");
    let text = std::fs::read_to_string(&snapshot).unwrap();
    std::fs::write(&snapshot, text.replace("\"total\":7", "\"total\":8")).unwrap();

    let reopened = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    assert_eq!(reopened.state().total, 7);
}

#[test]
fn a_lone_corrupt_snapshot_is_refused_rather_than_reset() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    log.commit(add(7)).unwrap();
    log.compact().unwrap();
    drop(log);
    std::fs::write(dir.path().join("snapshot.1.json"), b"{}").unwrap();

    let result = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD);
    assert!(result.is_err());
}

#[test]
fn seq_continues_across_compaction_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    let first = log.commit(add(1)).unwrap().seq;
    log.compact().unwrap();
    let second = log.commit(add(1)).unwrap().seq;
    assert_eq!(second, first + 1);
    let last = log.last_seq();
    drop(log);

    let mut reopened = RunLog::<Toy, _>::open(FileBackend::new(dir.path()), BUILD).unwrap();
    // The reopen's own `opened` record takes the next seq.
    assert_eq!(reopened.last_seq(), last + 1);
    let third = reopened.commit(add(1)).unwrap().seq;
    assert_eq!(third, last + 2);
    assert_eq!(reopened.state().total, 3);
}

#[test]
fn since_serves_records_and_falls_back_to_a_snapshot_after_compaction() {
    let mut log = RunLog::<Toy, _>::open(FaultBackend::new(), BUILD).unwrap();
    let a = log.commit(add(1)).unwrap().seq;
    log.commit(add(2)).unwrap();
    match log.since(a).unwrap() {
        CatchUp::Records(records) => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].seq, a + 1);
        }
        CatchUp::Snapshot { .. } => panic!("expected records"),
    }

    log.compact().unwrap();
    log.commit(add(3)).unwrap();
    match log.since(a).unwrap() {
        CatchUp::Snapshot { state, seq } => {
            assert_eq!(&state, log.state());
            assert_eq!(seq, log.last_seq());
        }
        CatchUp::Records(_) => panic!("expected a snapshot"),
    }
}

#[test]
fn should_compact_trips_past_the_record_budget() {
    let mut log = RunLog::<Toy, _>::open(FaultBackend::new(), BUILD).unwrap();
    assert!(!log.should_compact());
    for _ in 0..1100 {
        log.commit(add(0)).unwrap();
    }
    assert!(log.should_compact());
    log.compact().unwrap();
    assert!(!log.should_compact());
}
