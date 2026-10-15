use super::*;
use daruda_runlog::testing::{assert_machine_laws, enumerate_crashes};
use daruda_runlog::{RunLog, testing::FaultBackend};

fn claimed(channel: &str, id: InboxId) -> InboxEvent {
    InboxEvent::Claimed {
        channel: ChannelKey::new(channel),
        id,
    }
}

fn apply(state: &mut InboxState, event: &InboxEvent) {
    InboxMachine::check(state, event).unwrap();
    InboxMachine::apply(state, event);
}

#[test]
fn monotonic_claims_require_a_strictly_greater_id() {
    let channel = ChannelKey::new("telegram");
    let mut state = InboxState::default();
    apply(
        &mut state,
        &InboxEvent::Seeded {
            channel: channel.clone(),
            max: 40,
        },
    );
    for id in [39, 40] {
        assert!(matches!(
            InboxMachine::check(&state, &claimed("telegram", InboxId::Monotonic(id))),
            Err(InboxRejected::Duplicate { .. })
        ));
    }
    apply(&mut state, &claimed("telegram", InboxId::Monotonic(41)));
    assert_eq!(state.monotonic[&channel], 41);
    assert!(!state.monotonic.contains_key(&ChannelKey::new("other")));
}

#[test]
fn a_first_monotonic_claim_accepts_the_entire_id_range() {
    for id in [i64::MIN, -1, 0, i64::MAX] {
        let mut state = InboxState::default();
        apply(&mut state, &claimed("telegram", InboxId::Monotonic(id)));
        assert_eq!(state.monotonic[&ChannelKey::new("telegram")], id);
        assert!(matches!(
            InboxMachine::check(&state, &claimed("telegram", InboxId::Monotonic(id))),
            Err(InboxRejected::Duplicate { .. })
        ));
    }
}

#[test]
fn opaque_claims_keep_the_newest_256_ids_per_stream() {
    let mut state = InboxState::default();
    for id in 0..=INBOX_RING_CAPACITY {
        apply(
            &mut state,
            &claimed("slack", InboxId::Opaque(id.to_string())),
        );
    }
    let ring = &state.rings[&ChannelKey::new("slack")];
    assert_eq!(ring.len(), INBOX_RING_CAPACITY);
    assert_eq!(ring.front().unwrap(), "1");
    assert_eq!(ring.back().unwrap(), "256");
    assert!(InboxMachine::check(&state, &claimed("slack", InboxId::Opaque("0".into()))).is_ok());
    assert!(matches!(
        InboxMachine::check(&state, &claimed("slack", InboxId::Opaque("1".into()))),
        Err(InboxRejected::Duplicate { .. })
    ));
    assert!(InboxMachine::check(&state, &claimed("discord", InboxId::Opaque("1".into()))).is_ok());
}

#[test]
fn seeding_requires_no_history_and_explicit_removal_clears_both_maps() {
    let channel = ChannelKey::new("telegram");
    let seed = InboxEvent::Seeded {
        channel: channel.clone(),
        max: 10,
    };
    let mut state = InboxState {
        monotonic: BTreeMap::from([(channel.clone(), 9)]),
        rings: BTreeMap::from([(channel.clone(), VecDeque::from(["event".into()]))]),
        ..InboxState::default()
    };
    assert!(matches!(
        InboxMachine::check(&state, &seed),
        Err(InboxRejected::HasHistory { .. })
    ));
    apply(
        &mut state,
        &InboxEvent::ChannelRemoved {
            channel: channel.clone(),
        },
    );
    assert!(!state.monotonic.contains_key(&channel));
    assert!(!state.rings.contains_key(&channel));
    apply(&mut state, &seed);
    assert_eq!(state.monotonic[&channel], 10);
    let ring_only = InboxState {
        rings: BTreeMap::from([(channel, VecDeque::new())]),
        ..InboxState::default()
    };
    assert!(matches!(
        InboxMachine::check(&ring_only, &seed),
        Err(InboxRejected::HasHistory { .. })
    ));
}

#[test]
fn initialization_is_atomic_and_empty_initialization_is_not_pristine() {
    let mut state = InboxState::default();
    apply(
        &mut state,
        &InboxEvent::Initialized {
            seed: InboxSeed::default(),
        },
    );
    assert!(state.initialized);
    let again = InboxEvent::Initialized {
        seed: InboxSeed::default(),
    };
    assert_eq!(
        InboxMachine::check(&state, &again),
        Err(InboxRejected::CannotInitialize)
    );
    let history = InboxState {
        monotonic: BTreeMap::from([(ChannelKey::new("telegram"), 2)]),
        ..InboxState::default()
    };
    assert_eq!(
        InboxMachine::check(&history, &again),
        Err(InboxRejected::CannotInitialize)
    );
}

#[test]
fn initialization_rejects_oversized_or_duplicate_rings() {
    for ring in [
        (0..=INBOX_RING_CAPACITY).map(|id| id.to_string()).collect(),
        VecDeque::from(["event".into(), "event".into()]),
    ] {
        let event = InboxEvent::Initialized {
            seed: InboxSeed {
                rings: BTreeMap::from([(ChannelKey::new("slack"), ring)]),
                ..InboxSeed::default()
            },
        };
        assert_eq!(
            InboxMachine::check(&InboxState::default(), &event),
            Err(InboxRejected::InvalidRing)
        );
    }
}

#[test]
fn every_event_and_state_obeys_the_machine_laws_and_serializes() {
    let states = [
        InboxState::default(),
        InboxState {
            initialized: true,
            monotonic: BTreeMap::from([(ChannelKey::new("telegram"), 7)]),
            rings: BTreeMap::from([(ChannelKey::new("slack"), VecDeque::from(["a".into()]))]),
        },
    ];
    let events = vec![
        InboxEvent::Initialized {
            seed: InboxSeed::default(),
        },
        InboxEvent::Initialized {
            seed: InboxSeed {
                monotonic: BTreeMap::from([(ChannelKey::new("telegram"), 9)]),
                rings: BTreeMap::from([(ChannelKey::new("slack"), VecDeque::from(["b".into()]))]),
            },
        },
        InboxEvent::Seeded {
            channel: ChannelKey::new("telegram"),
            max: 5,
        },
        claimed("telegram", InboxId::Monotonic(7)),
        claimed("telegram", InboxId::Monotonic(8)),
        claimed("slack", InboxId::Opaque("a".into())),
        claimed("slack", InboxId::Opaque("b".into())),
        InboxEvent::ChannelRemoved {
            channel: ChannelKey::new("slack"),
        },
    ];
    for state in &states {
        assert_eq!(
            serde_json::from_slice::<InboxState>(&serde_json::to_vec(state).unwrap()).unwrap(),
            *state
        );
    }
    for event in &events {
        assert_eq!(
            serde_json::from_slice::<InboxEvent>(&serde_json::to_vec(event).unwrap()).unwrap(),
            *event
        );
    }
    assert_machine_laws::<InboxMachine>(&states, &events);
    assert!(enumerate_crashes::<InboxMachine>(events).cases > 0);
}

#[test]
fn a_whole_ring_import_survives_interruption_and_compaction_after_removal() {
    let channel = ChannelKey::new("slack");
    let imported = InboxEvent::Initialized {
        seed: InboxSeed {
            rings: BTreeMap::from([(
                channel.clone(),
                (0..INBOX_RING_CAPACITY).map(|id| id.to_string()).collect(),
            )]),
            ..InboxSeed::default()
        },
    };
    assert!(
        enumerate_crashes::<InboxMachine>(vec![
            imported.clone(),
            claimed("slack", InboxId::Opaque("new".into()))
        ])
        .cases
            > 0
    );
    for initialized in [
        imported,
        InboxEvent::Initialized {
            seed: InboxSeed::default(),
        },
    ] {
        let mut log = RunLog::<InboxMachine, _>::open(FaultBackend::new(), "test").unwrap();
        log.commit(initialized).unwrap();
        log.commit(InboxEvent::ChannelRemoved {
            channel: channel.clone(),
        })
        .unwrap();
        log.compact().unwrap();
        let reopened = RunLog::<InboxMachine, _>::open(log.backend().crash(), "test").unwrap();
        assert!(reopened.state().initialized);
        assert_eq!(
            InboxMachine::check(
                reopened.state(),
                &InboxEvent::Initialized {
                    seed: InboxSeed::default()
                }
            ),
            Err(InboxRejected::CannotInitialize)
        );
    }
}
