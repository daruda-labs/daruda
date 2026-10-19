use super::super::tests::{ended, running};
use super::*;
use crate::runs::result::{FlowRunLocation, RunKey};

fn locate(watch: &mut RunWatch, id: &str) {
    watch.locate(FlowRunLocation {
        run_dir: std::path::PathBuf::from("/runs").join(id),
        lock_dir: None,
    });
}

#[test]
fn a_key_whose_waiter_left_with_nothing_retained_is_forgotten_with_its_location() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("never".into());
    locate(&mut watch, "never");
    let (wait, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
    assert!(watch.entries.contains_key(&key));
    let answer = watch.poll_wait(wait, true).unwrap();
    assert_eq!(answer.outcome, super::super::WaitOutcome::TimedOut);
    watch.finish_wait(wait, true);
    assert!(!watch.entries.contains_key(&key));
    assert!(watch.location("never").is_none());
}

#[test]
fn reported_keys_are_remembered_up_to_the_cap_oldest_forgotten_first() {
    let mut watch = RunWatch::default();
    let fresh = RunKey::Flow("fresh".into());
    let report = |watch: &mut RunWatch, key: &RunKey| {
        watch.register(key.clone());
        watch.retain(ended(key.clone()));
        let (id, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
        assert_eq!(watch.poll_wait(id, false).unwrap().ended.len(), 1);
        watch.finish_wait(id, true);
    };
    watch.register(fresh.clone());
    let early = RunKey::Flow("early".into());
    report(&mut watch, &early);
    locate(&mut watch, "early");
    let flow = |n: usize| RunKey::Flow(format!("flow-{n}"));
    for n in 0..RETIRED_MAX + 250 {
        report(&mut watch, &flow(n));
    }
    assert!(
        watch.entries.contains_key(&fresh),
        "a registered start not yet observed stays in scope"
    );
    assert!(
        !watch.entries.contains_key(&early),
        "the oldest is forgotten"
    );
    assert!(watch.location("early").is_none());
    assert!(
        watch.entries.len() <= RETIRED_MAX + 201,
        "remembered keys, retained results and the open start: {}",
        watch.entries.len()
    );
    let evicted = flow(RETIRED_MAX);
    assert!(watch.get(&evicted).is_none(), "its result was evicted");
    watch.retain(ended(evicted.clone()));
    let (id, _) = watch.open_wait(Some(vec![evicted])).unwrap();
    assert_eq!(
        watch.poll_wait(id, false).unwrap().already_reported.len(),
        1,
        "a remembered key is not delivered again"
    );
}

#[test]
fn settled_observations_nobody_follows_are_bounded() {
    let mut watch = RunWatch::default();
    let followed = RunKey::Flow("followed".into());
    let live = RunKey::Flow("live".into());
    watch.register(followed.clone());
    watch.retain(ended(followed.clone()));
    watch.observe_waiting(running(live.clone(), None));
    for n in 0..(OBSERVATIONS_MAX + 500) {
        watch.observe_disk(ended(RunKey::Flow(format!("observed-{n}"))), 0);
    }
    watch.forget_idle();
    assert!(watch.entries.len() <= OBSERVATIONS_MAX + 2);
    assert!(watch.get(&followed).is_some(), "an unreported end stays");
    assert!(watch.get(&live).is_some(), "a running observation stays");
}

#[test]
fn reported_identities_do_not_evict_every_new_outside_observation() {
    let mut watch = RunWatch::default();
    for number in 0..RETIRED_MAX + 200 {
        let key = RunKey::Flow(format!("reported-{number}"));
        watch.register(key.clone());
        watch.retain(ended(key.clone()));
        let (wait, _) = watch.open_wait(Some(vec![key])).unwrap();
        watch.poll_wait(wait, false).unwrap();
        watch.finish_wait(wait, true);
    }
    let key = RunKey::Flow("outside-observation".into());
    locate(&mut watch, "outside-observation");
    watch.observe_disk(ended(key.clone()), 0);
    let live = RunKey::Flow("live".into());
    for number in 0..SWEEP_HEADROOM {
        let mut snapshot = running(live.clone(), None);
        snapshot.what = format!("progress-{number}");
        watch.observe_waiting(snapshot);
    }
    assert!(watch.get(&key).is_some());
    assert!(watch.location("outside-observation").is_some());
}

#[test]
fn unreported_local_flow_ends_keep_their_authority_under_observation_pressure() {
    let mut watch = RunWatch::default();
    let local = RunKey::Flow("local".into());
    let unconfirmed = RunKey::Flow("unconfirmed".into());
    for key in [&local, &unconfirmed] {
        watch.retain(ended(key.clone()));
    }
    watch.record_flow_sequence(local.clone(), 7);
    for number in 0..OBSERVATIONS_MAX + SWEEP_HEADROOM {
        watch.observe_disk(ended(RunKey::Flow(format!("disk-{number}"))), 0);
    }
    watch.forget_idle();
    for key in [&local, &unconfirmed] {
        assert_eq!(watch.get(key), Some(&ended(key.clone())));
        assert!(watch.local_end(key).is_some());
    }
    assert!(watch.local_end(&local).unwrap().superseded_by(8));
    assert!(!watch.local_end(&unconfirmed).unwrap().superseded_by(8));
}

#[test]
fn forgotten_keys_and_missing_keys_cannot_reuse_a_read_revision() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("recreated".into());
    let missing_revision = watch.revision(&key);
    let (wait, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
    let revision = watch.revision(&key);
    watch.finish_wait(wait, false);
    assert_ne!(watch.revision(&key), revision);
    assert_ne!(watch.revision(&key), missing_revision);
    let (_wait, _) = watch.open_wait(Some(vec![key.clone()])).unwrap();
    assert_ne!(watch.revision(&key), revision);
}
