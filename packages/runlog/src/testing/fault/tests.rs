use super::*;
use crate::backend::LogBackend as _;

fn selected(fault: Option<Fault>) -> FaultBackend {
    let mut backend = FaultBackend::new();
    if let Some(fault) = fault {
        backend = backend.with_fault(fault);
    }
    backend.select(0).unwrap();
    backend
}

#[test]
fn unsynced_bytes_do_not_survive_a_crash() {
    let mut backend = selected(None);
    backend.append(b"a\n").unwrap();
    backend.sync().unwrap();
    backend.append(b"b\n").unwrap();
    assert_eq!(backend.file("log.0.jsonl").unwrap(), b"a\nb\n");
    assert_eq!(backend.crash().file("log.0.jsonl").unwrap(), b"a\n");
}

#[test]
fn a_torn_append_leaves_its_prefix_on_disk_and_kills_the_backend() {
    let mut backend = selected(Some(Fault::TornAppend { k: 1, keep: 2 }));
    backend.append(b"a\n").unwrap();
    backend.sync().unwrap();
    assert!(backend.append(b"bcd\n").is_err());
    assert!(backend.sync().is_err());
    assert_eq!(backend.crash().file("log.0.jsonl").unwrap(), b"a\nbc");
}

#[test]
fn a_failed_sync_still_lands_the_line() {
    let mut backend = selected(Some(Fault::FailSyncAfter { k: 0 }));
    backend.append(b"a\n").unwrap();
    assert!(backend.sync().is_err());
    assert!(backend.append(b"b\n").is_err());
    assert_eq!(backend.crash().file("log.0.jsonl").unwrap(), b"a\n");
}

#[test]
fn after_a_power_cut_the_next_append_is_lost() {
    let mut backend = selected(Some(Fault::LoseUnsyncedAfter { k: 0 }));
    backend.append(b"a\n").unwrap();
    backend.sync().unwrap();
    assert!(backend.append(b"b\n").is_err());
    assert_eq!(backend.file("log.0.jsonl").unwrap(), b"a\nb\n");
    assert_eq!(backend.crash().file("log.0.jsonl").unwrap(), b"a\n");
}

#[test]
fn a_lost_snapshot_publication_refuses_cleanup_of_the_old_generation() {
    let mut backend = FaultBackend::new().lose_dir_ops_from(DirOp::ReplaceSnapshot(1));
    backend.select(0).unwrap();
    assert!(backend.replace_snapshot(1, b"s").is_err());
    assert!(backend.drop_generation(0).is_err());
    assert_eq!(backend.generations().unwrap(), vec![0, 1]);
    assert_eq!(backend.crash().generations().unwrap(), vec![0]);
}
