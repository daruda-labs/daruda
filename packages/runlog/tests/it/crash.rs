use daruda_runlog::testing::{Fault, FaultBackend, enumerate_crashes, enumerate_crashes_with};
use daruda_runlog::{CommitError, RunLog};

use crate::toy::{BUILD, Toy, add, scenario, ticket};

#[test]
fn every_crash_point_recovers_a_committed_prefix() {
    let summary = enumerate_crashes::<Toy>(scenario());
    // One opened record plus eight accepted events: nine appends, each
    // crossed with every fault kind.
    assert!(summary.cases >= 9 * 3, "only {} cases ran", summary.cases);
}

#[test]
fn every_fault_kind_is_exercised() {
    let (mut torn, mut failed_sync, mut lost) = (0, 0, 0);
    enumerate_crashes_with::<Toy>(scenario(), |case| match case.fault {
        Fault::TornAppend { .. } => torn += 1,
        Fault::FailSyncAfter { .. } => failed_sync += 1,
        Fault::LoseUnsyncedAfter { .. } => lost += 1,
    });
    assert!(torn > 0 && failed_sync > 0 && lost > 0);
}

#[test]
fn a_failed_sync_closes_the_log_without_applying() {
    let backend = FaultBackend::new().with_fault(Fault::FailSyncAfter { k: 1 });
    let mut log = RunLog::<Toy, _>::open(backend, BUILD).unwrap();
    assert!(matches!(log.commit(add(1)), Err(CommitError::Io(_))));
    assert_eq!(log.state().total, 0);
    assert!(matches!(log.commit(ticket()), Err(CommitError::Closed)));
    assert!(log.compact().is_err());
}
