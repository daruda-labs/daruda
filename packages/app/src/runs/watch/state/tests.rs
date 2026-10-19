use super::super::tests::{ended, running};
use super::*;

#[test]
fn retaining_an_identical_end_bumps_no_revision_and_wakes_no_waiter() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("same".into());
    watch.register(key.clone());
    watch.retain(ended(key.clone()));
    let revision = watch.revision(&key);
    let (_wait, wake) = watch
        .open_wait(Some(vec![RunKey::Flow("other".into())]))
        .unwrap();
    watch.retain(ended(key.clone()));
    assert_eq!(watch.revision(&key), revision);
    assert!(wake.try_recv().is_err());
    let mut changed = ended(key.clone());
    changed.what = "changed".into();
    watch.retain(changed);
    assert!(watch.revision(&key) > revision);
    assert_eq!(
        watch.unreported().len(),
        1,
        "a changed end is not a second episode"
    );
}

#[test]
fn a_held_end_reaches_waiters_only_when_released_with_its_enrichment() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("held".into());
    watch.register(key.clone());
    watch.observe_waiting(running(key.clone(), None));
    let (wait, _) = watch.open_wait(None).unwrap();
    let mut bare = ended(key.clone());
    if let RunState::Ended { text, .. } = &mut bare.state {
        *text = None;
    }
    watch.hold_end(bare.clone());
    assert!(watch.unreported().is_empty());
    assert!(
        watch.poll_wait(wait, false).is_none(),
        "a held end keeps the wait open"
    );
    let (snapshot, token) = watch.held_end(&key).unwrap();
    assert_eq!(snapshot, bare);
    watch.release_end(token.clone(), ended(key.clone()));
    let answer = watch.poll_wait(wait, false).unwrap();
    assert!(
        matches!(&answer.ended[0].state, RunState::Ended { text: Some(text), .. } if text == "answer")
    );
    watch.finish_wait(wait, true);
    watch.release_end(token, ended(key.clone()));
    assert!(
        watch.unreported().is_empty(),
        "a used token announces nothing"
    );
}

#[test]
fn a_held_end_is_released_by_a_later_retained_end_and_stale_tokens_do_nothing() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("fallback".into());
    watch.register(key.clone());
    let stale = watch.hold_end(ended(key.clone()));
    watch.retain(ended(key.clone()));
    assert_eq!(watch.unreported().len(), 1);
    watch.begin(&key);
    watch.hold_end(ended(key.clone()));
    watch.release_end(stale, ended(key.clone()));
    assert_eq!(
        watch.unreported().len(),
        1,
        "the previous episode's token cannot release the new end"
    );
}

#[test]
fn a_held_end_survives_an_immediate_resume_and_its_late_release_adds_its_text() {
    let mut watch = RunWatch::default();
    let key = RunKey::Flow("resumed".into());
    watch.register(key.clone());
    let mut bare = ended(key.clone());
    if let RunState::Ended { text, .. } = &mut bare.state {
        *text = None;
    }
    let token = watch.hold_end(bare);
    // The desktop resumes the run while its report is still being read.
    watch.begin(&key);
    watch.observe_waiting(running(key.clone(), None));
    watch.release_end(token.clone(), ended(key.clone()));
    let (wait, _) = watch.open_wait(None).unwrap();
    let answer = watch.poll_wait(wait, false).unwrap();
    assert_eq!(answer.ended.len(), 1, "the previous episode's end is kept");
    assert!(
        matches!(&answer.ended[0].state, RunState::Ended { text: Some(text), .. } if text == "answer"),
        "the late release still adds the report text"
    );
    watch.finish_wait(wait, true);
    assert!(matches!(
        watch.get(&key).unwrap().state,
        RunState::Running { .. }
    ));
    assert_eq!(
        watch.entries[&key].end,
        EndState::Open,
        "the new episode is unaffected"
    );
    watch.release_end(token, ended(key.clone()));
    assert!(watch.unreported().is_empty(), "delivered once");
    let (again, _) = watch.open_wait(None).unwrap();
    assert!(
        watch.poll_wait(again, false).is_none(),
        "the new episode is still running"
    );
}
