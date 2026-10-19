use super::super::tests::{ended, running};
use super::*;
use crate::runs::result::WaitingOn;

#[test]
fn releasing_the_last_explicit_waiter_drops_an_outside_enrollment() {
    let mut watch = RunWatch::default();
    let (tx, rx) = smol::channel::unbounded();
    watch.event_sender = Some(tx);
    let key = RunKey::Flow("outside".into());
    let (first, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
    let (second, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
    watch.retain(ended(key.clone()));
    assert!(
        rx.try_recv().is_err(),
        "an outside key never becomes a notice"
    );
    watch.finish_wait(first, false);
    assert_eq!(watch.membership(&key), Membership::Enrolled);
    watch.finish_wait(second, false);
    assert_eq!(watch.membership(&key), Membership::Outside);
    assert!(watch.unreported().is_empty());
    assert!(rx.try_recv().is_err());
    let (again, _) = watch.open_wait(Some(vec![key])).unwrap();
    assert_eq!(
        watch.poll_wait(again, false).unwrap().ended.len(),
        1,
        "a new enrollment recreates the undelivered end"
    );
}

#[test]
fn a_reported_outside_end_stays_reported_after_re_enrollment() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("outside".into());
    watch.retain(ended(key.clone()));
    let (wait, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
    watch.poll_wait(wait, false).unwrap();
    watch.finish_wait(wait, true);
    assert_eq!(watch.membership(&key), Membership::Outside);
    let (again, _) = watch.open_wait(Some(vec![key])).unwrap();
    let answer = watch.poll_wait(again, false).unwrap();
    assert!(answer.ended.is_empty());
    assert_eq!(answer.already_reported.len(), 1);
}

#[test]
fn a_released_outside_block_reopens_as_a_new_episode_when_named_again() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("blocked".into());
    let (wait, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
    watch.observe_waiting(running(
        key.clone(),
        Some(WaitingOn::Permission { tool: None }),
    ));
    let first = watch.pending(&key).unwrap().sequence;
    watch.finish_wait(wait, false);
    assert!(watch.pending(&key).is_none());
    let (again, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
    assert_eq!(watch.poll_wait(again, false).unwrap().blocked.len(), 1);
    assert!(watch.pending(&key).unwrap().sequence > first);
}

#[test]
fn scoped_keys_stay_in_the_ledger_after_an_explicit_wait() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("scoped".into());
    watch.register(key.clone());
    let (wait, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
    watch.finish_wait(wait, false);
    assert_eq!(watch.membership(&key), Membership::Scoped);
    watch.retain(ended(key));
    assert_eq!(watch.unreported().len(), 1);
}

#[test]
fn evicting_an_old_reported_block_preserves_the_held_end_across_resume() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("held-after-block".into());
    watch.register(key.clone());
    watch.observe_waiting(running(
        key.clone(),
        Some(WaitingOn::Question {
            node: "gate".into(),
        }),
    ));
    let (wait, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
    assert_eq!(watch.poll_wait(wait, false).unwrap().blocked.len(), 1);
    watch.finish_wait(wait, true);
    let mut bare = ended(key.clone());
    if let RunState::Ended { text, .. } = &mut bare.state {
        *text = None;
    }
    let held = watch.hold_end(bare.clone());
    for number in 0..REPORTED_MAX {
        let other = RunKey::Flow(format!("other-{number}"));
        watch.register(other.clone());
        watch.retain(ended(other.clone()));
        let (wait, _) = watch.open_wait(Some(vec![other])).unwrap();
        watch.poll_wait(wait, false).unwrap();
        watch.finish_wait(wait, true);
    }
    assert_eq!(watch.get(&key), Some(&bare));
    assert!(watch.held_end(&key).is_some());
    watch.begin(&key);
    watch.observe_waiting(running(key.clone(), None));
    watch.release_end(held, ended(key.clone()));
    let (wait, _) = watch.open_wait(None).unwrap();
    let answer = watch.poll_wait(wait, false).unwrap();
    assert_eq!(answer.ended, vec![ended(key)]);
}
