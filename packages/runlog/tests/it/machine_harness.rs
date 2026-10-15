use daruda_runlog::testing::{FaultBackend, assert_machine_laws};
use daruda_runlog::{CommitError, RunLog};

use crate::toy::{
    BUILD, LIMIT, Toy, ToyEvent, ToyRejected, ToyState, add, insert, remove, scenario, ticket,
};

fn sample_states() -> Vec<ToyState> {
    let mut busy = ToyState {
        total: LIMIT - 1,
        ..ToyState::default()
    };
    busy.members.insert("a".into());
    busy.tickets = vec![1, 2, 3];
    busy.next_ticket = 3;
    vec![ToyState::default(), busy]
}

#[test]
fn the_toy_machine_obeys_the_laws() {
    let mut events = scenario();
    events.extend([add(LIMIT), remove("zz"), insert("a")]);
    assert_machine_laws::<Toy>(&sample_states(), &events);
}

#[test]
fn assign_fills_the_number_the_log_owns() {
    let mut log = RunLog::<Toy, _>::open(FaultBackend::new(), BUILD).unwrap();
    let first = log.commit(ticket()).unwrap();
    let second = log.commit(ticket()).unwrap();
    assert_eq!(first.event, ToyEvent::Ticket { number: 1 });
    assert_eq!(second.event, ToyEvent::Ticket { number: 2 });
    assert_eq!(second.state_after.tickets, vec![1, 2]);
    assert_eq!(log.state(), &second.state_after);
}

#[test]
fn a_rejected_event_writes_nothing_and_keeps_the_seq() {
    let mut log = RunLog::<Toy, _>::open(FaultBackend::new(), BUILD).unwrap();
    let before = log.last_seq();
    let err = log.commit(remove("nobody")).unwrap_err();
    assert!(matches!(
        err,
        CommitError::Rejected(ToyRejected::Missing(_))
    ));
    assert_eq!(log.last_seq(), before);
    assert_eq!(log.state(), &ToyState::default());
}
