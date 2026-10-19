use super::*;
use crate::runs::result::{RunKey, RunSnapshot, RunState};

#[test]
fn reported_cap_evicts_oldest_first_and_never_unreported_or_in_flight_work() {
    use super::delivery::Delivery;
    let mut watch = RunWatch::default();
    let flow = |n: usize| RunKey::Flow(format!("flow-{n}"));
    let unreported = RunKey::Flow("unreported".into());
    let in_flight = RunKey::Flow("in-flight".into());
    for key in [&unreported, &in_flight] {
        watch.register(key.clone());
        watch.retain(ended(key.clone()));
    }
    let (held, _) = watch.open_wait(Some(vec![in_flight.clone()])).unwrap();
    assert_eq!(watch.poll_wait(held, false).unwrap().ended.len(), 1);
    for n in 0..250 {
        let key = flow(n);
        watch.register(key.clone());
        watch.retain(ended(key.clone()));
        let (id, _) = watch.open_wait(Some(vec![key])).unwrap();
        assert_eq!(watch.poll_wait(id, false).unwrap().ended.len(), 1);
        watch.finish_wait(id, true);
    }
    let reported = |watch: &RunWatch| {
        watch
            .events
            .iter()
            .filter(|event| event.delivery == Delivery::Reported)
            .count()
    };
    assert_eq!(reported(&watch), 200);
    for n in 0..50 {
        assert!(
            watch.get(&flow(n)).is_none(),
            "the oldest result {n} is evicted"
        );
    }
    for n in 50..250 {
        assert!(watch.get(&flow(n)).is_some(), "the newest 200 stay");
    }
    assert!(watch.get(&unreported).is_some());
    assert!(watch.get(&in_flight).is_some());
    assert_eq!(watch.unreported().len(), 1);
    watch.finish_wait(held, true);
    assert_eq!(reported(&watch), 200);
    assert!(
        watch.get(&in_flight).is_none(),
        "once reported, the earliest episode is the oldest"
    );
    assert!(watch.get(&flow(50)).is_some());
    assert!(watch.get(&unreported).is_some());
}

pub(super) fn running(
    key: RunKey,
    waiting_on: Option<crate::runs::result::WaitingOn>,
) -> RunSnapshot {
    RunSnapshot {
        key,
        what: "run".into(),
        why: None,
        state: RunState::Running {
            progress: Default::default(),
            waiting_on,
        },
    }
}

#[test]
fn blocked_levels_close_and_reopen_as_distinct_episodes() {
    use crate::runs::result::WaitingOn;
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("flow".into());
    watch.register(key.clone());
    let wait = Some(WaitingOn::Question { node: "a".into() });
    watch.observe_waiting(running(key.clone(), wait.clone()));
    let first = watch.pending(&key).unwrap().sequence;
    watch.observe_waiting(running(key.clone(), wait));
    assert_eq!(watch.pending(&key).unwrap().sequence, first);
    watch.observe_waiting(running(
        key.clone(),
        Some(WaitingOn::Question { node: "b".into() }),
    ));
    assert_eq!(watch.pending(&key).unwrap().sequence, first);
    watch.observe_waiting(running(key.clone(), None));
    assert!(watch.pending(&key).is_none());
    watch.observe_waiting(running(
        key.clone(),
        Some(WaitingOn::Question { node: "c".into() }),
    ));
    assert!(watch.pending(&key).unwrap().sequence > first);
}

#[test]
fn terminal_supersedes_blocked_without_regenerating_on_history_projection() {
    use crate::runs::result::{EndedAs, WaitingOn};
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("flow".into());
    watch.register(key.clone());
    watch.observe_waiting(running(
        key.clone(),
        Some(WaitingOn::Permission { tool: None }),
    ));
    let blocked = watch.pending(&key).unwrap().sequence;
    let end = RunSnapshot {
        key: key.clone(),
        what: "end".into(),
        why: None,
        state: RunState::Ended {
            ended_as: EndedAs::Completed,
            text: None,
            pointers: Default::default(),
        },
    };
    watch.retain(end.clone());
    let terminal = watch.pending(&key).unwrap().sequence;
    assert!(terminal > blocked);
    assert!(watch.waiting_on(&key).is_none());
    watch.retain(end.clone());
    watch.enrich(end);
    assert_eq!(watch.pending(&key).unwrap().sequence, terminal);
}

#[test]
fn outside_scope_is_not_an_unreported_event() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("outside".into());
    watch.observe_waiting(running(
        key.clone(),
        Some(crate::runs::result::WaitingOn::Permission { tool: None }),
    ));
    assert!(watch.pending(&key).is_none());
}

#[test]
fn resumed_flow_can_end_again_under_the_same_handle() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("resumed".into());
    watch.register(key.clone());
    let end = RunSnapshot {
        key: key.clone(),
        what: "run".into(),
        why: None,
        state: RunState::Ended {
            ended_as: crate::runs::result::EndedAs::Completed,
            text: None,
            pointers: Default::default(),
        },
    };
    watch.retain(end.clone());
    let first = watch.pending(&key).unwrap().sequence;
    watch.begin(&key);
    watch.retain(end);
    assert!(watch.pending(&key).unwrap().sequence > first);
}

#[test]
fn episode_numbers_advance_independently_for_each_run_key() {
    let mut watch = RunWatch::default();
    for id in ["first", "second"] {
        let key = RunKey::Flow(id.into());
        watch.register(key.clone());
        watch.observe_waiting(running(
            key.clone(),
            Some(crate::runs::result::WaitingOn::Permission { tool: None }),
        ));
        assert_eq!(watch.pending(&key).unwrap().sequence, 1);
    }
}

#[test]
fn registering_a_cached_ended_start_creates_its_delivery_event() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("finished-before-start-returned".into());
    watch.retain(RunSnapshot {
        key: key.clone(),
        what: "fast".into(),
        why: None,
        state: RunState::Ended {
            ended_as: crate::runs::result::EndedAs::Completed,
            text: None,
            pointers: Default::default(),
        },
    });
    assert!(watch.pending(&key).is_none());
    watch.register(key.clone());
    assert!(
        watch.pending(&key).is_some(),
        "successful fast starts must be reportable"
    );
}

pub(super) fn ended(key: RunKey) -> RunSnapshot {
    RunSnapshot {
        key,
        what: "ended".into(),
        why: None,
        state: RunState::Ended {
            ended_as: crate::runs::result::EndedAs::Completed,
            text: Some("answer".into()),
            pointers: Default::default(),
        },
    }
}

#[test]
fn reservations_rollback_and_late_ack_cannot_consume_a_new_wait() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("f".into());
    watch.register(key.clone());
    watch.retain(ended(key.clone()));
    let (a, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
    let (b, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
    assert_eq!(watch.poll_wait(a, false).unwrap().ended.len(), 1);
    assert!(watch.poll_wait(b, false).is_none());
    watch.finish_wait(a, false);
    assert_eq!(watch.poll_wait(b, false).unwrap().ended.len(), 1);
    watch.finish_wait(a, true);
    assert!(watch.unreported().is_empty());
    watch.finish_wait(b, true);
    let (c, _) = watch.open_wait(Some(vec![key])).unwrap();
    assert_eq!(watch.poll_wait(c, false).unwrap().already_reported.len(), 1);
}

#[test]
fn explicit_reported_a_waits_for_b_and_remains_in_timeout_and_settled_answers() {
    let mut watch = RunWatch::default();
    let a = RunKey::Flow("a".into());
    let b = RunKey::Flow("b".into());
    watch.register(a.clone());
    watch.retain(ended(a.clone()));
    watch.retain(running(b.clone(), None));
    let (first, _) = watch.open_wait(Some(vec![a.clone()])).unwrap();
    watch.poll_wait(first, false).unwrap();
    watch.finish_wait(first, true);
    let (wait, _) = watch.open_wait(Some(vec![a, b.clone()])).unwrap();
    assert!(watch.poll_wait(wait, false).is_none());
    let timeout = watch.poll_wait(wait, true).unwrap();
    assert_eq!(timeout.outcome, WaitOutcome::TimedOut);
    assert_eq!(timeout.already_reported.len(), 1);
    assert_eq!(timeout.pending.len(), 1);
    watch.retain(ended(b));
    let answer = watch.poll_wait(wait, false).unwrap();
    assert_eq!(answer.outcome, WaitOutcome::Settled);
    assert_eq!(answer.ended.len(), 1);
    assert_eq!(answer.already_reported.len(), 1);
}

#[test]
fn fifo_ended_first_batches_twenty_with_more_and_omitted_waiter_is_exclusive() {
    let mut watch = RunWatch::default();
    let block = RunKey::Flow("block".into());
    watch.register(block.clone());
    watch.observe_waiting(running(
        block,
        Some(crate::runs::result::WaitingOn::Permission { tool: None }),
    ));
    for n in 0..22 {
        let key = RunKey::Flow(n.to_string());
        watch.register(key.clone());
        watch.retain(ended(key));
    }
    let (wait, _) = watch.open_wait(None).unwrap();
    assert!(watch.open_wait(None).is_err());
    let first = watch.poll_wait(wait, false).unwrap();
    assert_eq!(first.ended.len(), 20);
    assert!(first.blocked.is_empty());
    assert_eq!(first.more, 3);
    assert_eq!(first.ended[0].key, RunKey::Flow("0".into()));
    watch.finish_wait(wait, true);
    let (wait, _) = watch.open_wait(None).unwrap();
    let second = watch.poll_wait(wait, false).unwrap();
    assert_eq!(second.ended.len(), 2);
    assert_eq!(second.blocked.len(), 1);
    assert_eq!(second.more, 0);
}

#[test]
fn closed_or_terminal_block_preserves_in_flight_identity_until_rollback_or_ack() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("block".into());
    watch.register(key.clone());
    watch.observe_waiting(running(
        key.clone(),
        Some(crate::runs::result::WaitingOn::Permission { tool: None }),
    ));
    let (id, _) = watch.open_wait(None).unwrap();
    let answer = watch.poll_wait(id, false).unwrap();
    assert_eq!(answer.blocked.len(), 1);
    watch.retain(ended(key.clone()));
    assert_eq!(watch.events.len(), 2);
    watch.finish_wait(id, false);
    assert_eq!(watch.events.len(), 1);
    let (next, _) = watch.open_wait(None).unwrap();
    assert_eq!(watch.poll_wait(next, false).unwrap().ended.len(), 1);
}

#[test]
fn explicit_outside_scope_enrolls_cached_end_and_blocks_without_expanding_scope() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("outside".into());
    watch.retain(ended(key.clone()));
    let (id, _) = watch.open_wait(Some(vec![key])).unwrap();
    assert!(watch.scope().is_empty());
    assert_eq!(watch.poll_wait(id, false).unwrap().ended.len(), 1);
}

#[test]
fn unreported_and_in_flight_results_survive_cap_and_text_enrichment_updates_event() {
    let mut watch = RunWatch::default();
    for n in 0..250 {
        let key = RunKey::Flow(n.to_string());
        watch.register(key.clone());
        watch.retain(ended(key));
    }
    let key = RunKey::Flow("0".into());
    let mut enriched = ended(key.clone());
    if let RunState::Ended { text, .. } = &mut enriched.state {
        *text = Some("enriched".into());
    }
    watch.enrich(enriched);
    let (id, _) = watch.open_wait(None).unwrap();
    let result = watch.poll_wait(id, false).unwrap();
    assert!(
        matches!(&result.ended[0].state, RunState::Ended { text: Some(t), .. } if t == "enriched")
    );
    assert_eq!(watch.retained_count(), 250);
    assert_eq!(watch.events.len(), 250);
    watch.finish_wait(id, false);
    assert_eq!(watch.unreported().len(), 250);
    while !watch.unreported().is_empty() {
        let (id, _) = watch.open_wait(None).unwrap();
        watch.poll_wait(id, false).unwrap();
        watch.finish_wait(id, true);
    }
    assert_eq!(watch.events.len(), 200);
    assert_eq!(watch.retained_count(), 200);
    assert!(watch.get(&key).is_none());
    let (empty, _) = watch.open_wait(None).unwrap();
    assert_eq!(
        watch.poll_wait(empty, false).unwrap().outcome,
        WaitOutcome::Nothing
    );
    watch.finish_wait(empty, true);
    watch.retain(ended(key.clone()));
    assert!(
        watch.unreported().is_empty(),
        "pruned reported results must not regenerate"
    );
    let (old, _) = watch.open_wait(Some(vec![key])).unwrap();
    assert_eq!(
        watch.poll_wait(old, false).unwrap().already_reported.len(),
        1
    );
    watch.finish_wait(old, true);
    assert_eq!(
        watch.retained_count(),
        200,
        "rehydrating a reported handle cannot grow the cache"
    );
}

#[test]
fn resumed_flow_waits_for_new_episode_instead_of_returning_old_reported_end() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("resumed".into());
    watch.register(key.clone());
    watch.retain(ended(key.clone()));
    let (id, _) = watch.open_wait(None).unwrap();
    watch.poll_wait(id, false).unwrap();
    watch.finish_wait(id, true);
    watch.begin(&key);
    watch.observe_waiting(running(key.clone(), None));
    let (id, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
    assert!(watch.poll_wait(id, false).is_none());
    watch.retain(ended(key));
    assert_eq!(watch.poll_wait(id, false).unwrap().ended.len(), 1);
}

#[test]
fn event_arrival_wakes_waiters_and_notice_filter_excludes_reservations_and_coverage() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("f".into());
    watch.register(key.clone());
    watch.retain(running(key.clone(), None));
    let (id, wake) = watch.open_wait(None).unwrap();
    assert!(watch.covered(&key));
    watch.retain(ended(key.clone()));
    assert!(wake.try_recv().is_ok());
    assert_eq!(watch.unreported().len(), 1);
    watch.poll_wait(id, false).unwrap();
    assert!(watch.unreported().is_empty());
    assert!(watch.covered(&key));
    watch.finish_wait(id, false);
    assert!(!watch.covered(&key));
    assert_eq!(watch.unreported().len(), 1);
}

#[test]
fn rolling_back_a_closed_episode_cannot_revive_it_after_a_new_block_opens() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("blocked".into());
    watch.register(key.clone());
    let blocked = || {
        running(
            key.clone(),
            Some(crate::runs::result::WaitingOn::Permission { tool: None }),
        )
    };
    watch.observe_waiting(blocked());
    let (id, _) = watch.open_wait(None).unwrap();
    watch.poll_wait(id, false).unwrap();
    watch.observe_waiting(running(key.clone(), None));
    watch.observe_waiting(blocked());
    watch.finish_wait(id, false);
    assert_eq!(
        watch.unreported().len(),
        1,
        "only the current blocked episode can be redelivered"
    );
}

#[test]
fn explicit_flow_handle_selects_only_latest_episode_after_resume() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("flow".into());
    watch.register(key.clone());
    watch.retain(ended(key.clone()));
    watch.begin(&key);
    let mut latest = ended(key.clone());
    latest.what = "latest".into();
    watch.retain(latest);
    let (id, _) = watch.open_wait(Some(vec![key])).unwrap();
    let answer = watch.poll_wait(id, false).unwrap();
    assert_eq!(answer.ended.len(), 1);
    assert_eq!(answer.ended[0].what, "latest");
}

#[test]
fn resumed_flow_text_enrichment_updates_only_its_current_episode() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("flow".into());
    let mut first = ended(key.clone());
    if let RunState::Ended { text, .. } = &mut first.state {
        *text = None;
    }
    watch.register(key.clone());
    watch.retain(first.clone());
    watch.begin(&key);
    watch.retain(first);
    let mut enriched = ended(key);
    if let RunState::Ended { text, .. } = &mut enriched.state {
        *text = Some("latest answer".into());
    }
    watch.enrich(enriched);
    let (id, _) = watch.open_wait(None).unwrap();
    let answer = watch.poll_wait(id, false).unwrap();
    assert_eq!(answer.ended.len(), 2);
    assert!(matches!(
        answer.ended[0].state,
        RunState::Ended { text: None, .. }
    ));
    assert!(
        matches!(&answer.ended[1].state, RunState::Ended { text: Some(text), .. } if text == "latest answer")
    );
}

#[test]
fn waiter_release_reannounces_stable_events_after_pruning_closed_blocks() {
    let mut watch = RunWatch::default();
    let (tx, rx) = smol::channel::unbounded();
    watch.event_sender = Some(tx);
    let key = RunKey::Flow("available".into());
    watch.register(key.clone());
    watch.observe_waiting(running(
        key.clone(),
        Some(crate::runs::result::WaitingOn::Permission { tool: None }),
    ));
    let first = rx.try_recv().unwrap();
    let (id, _) = watch.open_wait(None).unwrap();
    watch.poll_wait(id, false).unwrap();
    watch.observe_waiting(running(key.clone(), None));
    watch.retain(ended(key.clone()));
    let terminal = rx.try_recv().unwrap();
    watch.finish_wait(id, false);
    let available = rx.try_recv().unwrap();
    assert_eq!(available.sequence, terminal.sequence);
    assert_eq!(available.snapshot, terminal.snapshot);
    assert_ne!(available.sequence, first.sequence);
    assert!(
        rx.try_recv().is_err(),
        "the closed block must not be announced again"
    );
    let (id, _) = watch.open_wait(Some(vec![key])).unwrap();
    watch.poll_wait(id, false).unwrap();
    watch.finish_wait(id, true);
    assert!(
        rx.try_recv().is_err(),
        "a reported event is no longer available"
    );
}

#[test]
fn successful_receipt_announces_the_remaining_unselected_events() {
    let mut watch = RunWatch::default();
    let (tx, rx) = smol::channel::unbounded();
    watch.event_sender = Some(tx);
    for index in 0..21 {
        let key = RunKey::Flow(format!("flow-{index}"));
        watch.register(key.clone());
        watch.retain(ended(key));
        rx.try_recv().unwrap();
    }
    let (id, _) = watch.open_wait(None).unwrap();
    let answer = watch.poll_wait(id, false).unwrap();
    assert_eq!(answer.ended.len(), 20);
    assert_eq!(answer.more, 1);
    let remaining = watch.unreported().pop().unwrap();
    watch.finish_wait(id, true);
    let available = rx.try_recv().unwrap();
    assert_eq!(available.sequence, remaining.sequence);
    assert_eq!(available.snapshot, remaining.snapshot);
    assert!(rx.try_recv().is_err());
}
