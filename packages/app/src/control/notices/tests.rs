use super::*;
use crate::runs::result::{EndedAs, RunSnapshot, RunState, WaitingOn};
use gpui::{EntityId, TestAppContext};

fn audience(pane: u64, session: Option<&str>) -> Audience {
    Audience::new(EntityId::from(pane), session)
}
fn ended(id: &str) -> RunSnapshot {
    RunSnapshot {
        key: RunKey::Flow(id.into()),
        what: id.into(),
        why: None,
        state: RunState::Ended {
            ended_as: EndedAs::Completed,
            text: None,
            pointers: Default::default(),
        },
    }
}
fn produce(id: &str, cx: &mut gpui::App) {
    let snapshot = ended(id);
    RunsHost::register_run(snapshot.key.clone(), cx);
    RunsHost::seed_run_wait(snapshot, cx);
}

#[gpui::test]
fn a_batch_is_solicited_if_any_eligible_episode_was_solicited(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        produce("desktop", cx);
        let snapshot = ended("phone");
        RunsHost::register_run_solicited(snapshot.key.clone(), true, cx);
        RunsHost::seed_run_wait(snapshot, cx);
        let mixed = batch(audience(1, None), cx).unwrap();
        assert!(mixed.solicited);
        assert_eq!(mixed.episodes.len(), 2);
        let (wait, _) =
            RunsHost::open_run_wait(Some(vec![RunKey::Flow("phone".into())]), cx).unwrap();
        let desktop = batch(audience(1, None), cx).unwrap();
        assert!(!desktop.solicited);
        assert_eq!(desktop.episodes.len(), 1);
        RunsHost::finish_run_wait(wait, false, cx);
    });
}

fn blocked(id: &str, waiting_on: WaitingOn) -> RunSnapshot {
    RunSnapshot {
        key: RunKey::Flow(id.into()),
        what: "title\nwith\u{1b} controls".into(),
        why: None,
        state: RunState::Running {
            progress: Default::default(),
            waiting_on: Some(waiting_on),
        },
    }
}

#[gpui::test]
fn question_and_permission_pointer_fields_stay_on_one_line(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        for (id, waiting) in [
            (
                "question",
                WaitingOn::Question {
                    node: "node\nquestion\u{2028}text".into(),
                },
            ),
            (
                "permission",
                WaitingOn::Permission {
                    tool: Some("tool\tname\u{2029}text".into()),
                },
            ),
        ] {
            let snapshot = blocked(id, waiting);
            RunsHost::register_run(snapshot.key.clone(), cx);
            RunsHost::seed_run_wait(snapshot, cx);
        }
        let batch = batch(audience(1, Some("s")), cx).unwrap();
        assert_eq!(batch.prompt.lines().count(), 2);
        assert!(batch.prompt.contains("node question text"));
        assert!(batch.prompt.contains("tool name text"));
        assert!(!batch.prompt.contains('\u{1b}'));
        accept(batch, cx);
        assert!(self::batch(audience(1, Some("s")), cx).is_none());
        assert_eq!(RunsHost::unreported_run_events(cx).len(), 2);
    });
}

#[gpui::test]
fn covered_events_remain_eligible_after_another_batch_is_accepted(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        for id in ["covered", "free"] {
            produce(id, cx);
        }
        let (wait, _) =
            RunsHost::open_run_wait(Some(vec![RunKey::Flow("covered".into())]), cx).unwrap();
        let first = batch(audience(1, None), cx).unwrap();
        assert_eq!(first.episodes.len(), 1);
        assert!(first.prompt.starts_with("flow:free"));
        accept(first, cx);
        RunsHost::finish_run_wait(wait, false, cx);
        let next = batch(audience(1, None), cx).unwrap();
        assert_eq!(next.episodes.len(), 1);
        assert!(next.prompt.starts_with("flow:covered"));
    });
}

#[gpui::test]
fn a_replaced_session_is_told_what_only_its_predecessor_heard(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        produce("told", cx);
        accept(batch(audience(1, Some("old")), cx).unwrap(), cx);
        assert!(batch(audience(1, Some("old")), cx).is_none());
        let renewed = batch(audience(1, Some("new")), cx).unwrap();
        assert!(renewed.prompt.starts_with("flow:told"));
        assert!(
            batch(audience(2, Some("old")), cx).is_some(),
            "another pane"
        );
        accept(renewed, cx);
        assert!(batch(audience(1, Some("new")), cx).is_none());
        assert_eq!(told_count(cx), 1, "the old session's record is replaced");
    });
}

#[gpui::test]
fn reported_episodes_leave_the_told_record(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        produce("reported", cx);
        produce("pending", cx);
        accept(batch(audience(1, None), cx).unwrap(), cx);
        assert_eq!(told_count(cx), 2);
        let (wait, _) =
            RunsHost::open_run_wait(Some(vec![RunKey::Flow("reported".into())]), cx).unwrap();
        RunsHost::poll_run_wait(wait, false, cx).unwrap();
        RunsHost::finish_run_wait(wait, true, cx);
        assert!(batch(audience(1, None), cx).is_none());
        assert_eq!(told_count(cx), 1);
    });
}
