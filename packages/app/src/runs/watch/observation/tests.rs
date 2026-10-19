use super::super::tests::ended;
use super::*;

#[test]
fn an_older_disk_episode_cannot_replace_a_newer_one() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("disk".into());
    watch.register(key.clone());
    let mut newer = ended(key.clone());
    newer.what = "newer".into();
    watch.observe_disk(newer, 5);
    let mut older = ended(key.clone());
    older.what = "older".into();
    watch.observe_disk(older, 3);
    assert_eq!(watch.get(&key).unwrap().what, "newer");
    assert_eq!(watch.unreported().len(), 1);
}

#[test]
fn a_resume_past_the_recorded_local_end_starts_a_new_episode() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("resumed".into());
    watch.register(key.clone());
    watch.retain(ended(key.clone()));
    watch.record_flow_sequence(key.clone(), 7);
    assert!(matches!(
        watch.local_end(&key),
        Some(LocalEnd::RecordedThrough(7))
    ));
    let first = watch.pending(&key).unwrap().sequence;
    watch.observe_disk(ended(key.clone()), 9);
    assert!(watch.pending(&key).unwrap().sequence > first);
    assert!(
        watch.local_end(&key).is_none(),
        "the disk now speaks for the run"
    );
}
