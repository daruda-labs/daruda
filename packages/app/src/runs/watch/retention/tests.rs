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
    for n in 0..ENTRIES_MAX + 250 {
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
        watch.entries.len() <= ENTRIES_MAX + 201,
        "remembered keys, retained results and the open start: {}",
        watch.entries.len()
    );
    let evicted = flow(ENTRIES_MAX);
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
    for n in 0..(ENTRIES_MAX + 500) {
        watch.retain(ended(RunKey::Flow(format!("observed-{n}"))));
    }
    assert!(watch.entries.len() <= ENTRIES_MAX + 1);
    assert!(watch.get(&followed).is_some(), "an unreported end stays");
    assert!(watch.get(&live).is_some(), "a running observation stays");
}
