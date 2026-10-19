use super::*;
use crate::runs::result::{EndedAs, RunSnapshot, RunState, WaitingOn};
use gpui::TestAppContext;

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
        let batch = batch(cx).unwrap();
        assert_eq!(batch.prompt.lines().count(), 2);
        assert!(batch.prompt.contains("node question text"));
        assert!(batch.prompt.contains("tool name text"));
        assert!(!batch.prompt.contains('\u{1b}'));
        accept(batch, cx);
        assert!(self::batch(cx).is_none());
        assert_eq!(RunsHost::unreported_run_events(cx).len(), 2);
    });
}

#[gpui::test]
fn covered_events_remain_eligible_after_another_batch_is_accepted(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        for id in ["covered", "free"] {
            let snapshot = RunSnapshot {
                key: RunKey::Flow(id.into()),
                what: id.into(),
                why: None,
                state: RunState::Ended {
                    ended_as: EndedAs::Completed,
                    text: None,
                    pointers: Default::default(),
                },
            };
            RunsHost::register_run(snapshot.key.clone(), cx);
            RunsHost::seed_run_wait(snapshot, cx);
        }
        let (wait, _) =
            RunsHost::open_run_wait(Some(vec![RunKey::Flow("covered".into())]), cx).unwrap();
        let first = batch(cx).unwrap();
        assert_eq!(first.episodes.len(), 1);
        assert!(first.prompt.starts_with("flow:free"));
        accept(first, cx);
        RunsHost::finish_run_wait(wait, false, cx);
        let next = batch(cx).unwrap();
        assert_eq!(next.episodes.len(), 1);
        assert!(next.prompt.starts_with("flow:covered"));
    });
}
