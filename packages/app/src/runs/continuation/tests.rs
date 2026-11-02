use daruda_store::project::WorkspaceUuid;
use daruda_store::tasks::{Lifecycle, StartedBy};
use gpui::TestAppContext;

use super::super::seed::{self, SeedEnd, SeedRun};
use super::super::{GlobalTasks, RunPane, RunsHost};

const SESSION: &str = "session-1";

fn pane() -> RunPane {
    RunPane {
        workspace: WorkspaceUuid::new(),
        pane: 7,
    }
}

/// A task whose Agent Chat run on [`SESSION`] stopped at `end`, and that run.
fn ended(end: SeedEnd, cx: &mut TestAppContext) -> (String, SeedRun) {
    cx.update(|cx| {
        RunsHost::inline_commits_for_test(cx);
        let run = SeedRun::chat("/lane").session(SESSION);
        let id = seed::task(
            seed::spec(Default::default(), "Fix login"),
            Some((&run, end)),
            cx,
        );
        (id, run)
    })
}

fn lifecycle(id: &str, cx: &mut TestAppContext) -> Lifecycle {
    cx.update(|cx| {
        cx.global::<GlobalTasks>()
            .get(id)
            .unwrap()
            .lifecycle
            .clone()
    })
}

fn continue_in(pane: RunPane, session: &str, cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::person_prompt_sent(
            pane,
            Some(session.into()),
            StartedBy::Desktop,
            "also fix the tests".into(),
            cx,
        )
    });
    cx.run_until_parked();
}

/// A person speaking again in the pane a finished run used starts a new run
/// of the same task, and the pane follows it so the next turn settles it.
#[gpui::test]
fn a_prompt_in_the_runs_pane_continues_the_task(cx: &mut TestAppContext) {
    let (id, run) = ended(SeedEnd::Done, cx);
    let pane = pane();
    cx.update(|cx| RunsHost::bind_execution(pane, run.exec(&id), cx));
    continue_in(pane, SESSION, cx);

    let Lifecycle::Running { exec, .. } = lifecycle(&id, cx) else {
        panic!("the task runs again");
    };
    assert_ne!(exec, run.exec(&id), "as a new run");
    cx.update(|cx| {
        assert_eq!(RunsHost::pane_execution(pane, cx), Some(&exec));
        let record = cx.global::<GlobalTasks>().get(&id).unwrap();
        assert_eq!(record.started_by, Some(StartedBy::Desktop));
        let description = record.description.as_ref().unwrap();
        assert!(description.continued);
        assert_eq!(description.prompt, "also fix the tests");
        assert_eq!(record.ended_executions.len(), 1, "the first run is kept");
    });
}

/// A pane restored after a restart has no binding; the session the run
/// recorded is what ties it to the task.
#[gpui::test]
fn a_restored_pane_continues_by_its_session(cx: &mut TestAppContext) {
    let (id, run) = ended(SeedEnd::Interrupted, cx);
    let pane = pane();
    continue_in(pane, SESSION, cx);

    let Lifecycle::Running { exec, .. } = lifecycle(&id, cx) else {
        panic!("a stopped run is continued too");
    };
    assert_ne!(exec, run.exec(&id));
    cx.update(|cx| assert_eq!(RunsHost::pane_execution(pane, cx), Some(&exec)));
}

/// A failed run is retried, not continued; another session is another
/// conversation.
#[gpui::test]
fn only_the_same_conversation_after_a_chosen_stop_continues(cx: &mut TestAppContext) {
    let (failed, run) = ended(SeedEnd::Errored, cx);
    let pane_a = pane();
    cx.update(|cx| RunsHost::bind_execution(pane_a, run.exec(&failed), cx));
    continue_in(pane_a, SESSION, cx);
    assert!(matches!(lifecycle(&failed, cx), Lifecycle::Error { .. }));

    let (done, _) = ended(SeedEnd::Done, cx);
    continue_in(pane(), "another-session", cx);
    assert!(matches!(lifecycle(&done, cx), Lifecycle::Done { .. }));
}

/// The continued run settles like any run, and the task's first result stays
/// beside the new one.
#[gpui::test]
fn the_continued_run_settles_with_its_own_result(cx: &mut TestAppContext) {
    let (id, run) = ended(SeedEnd::Done, cx);
    let pane = pane();
    cx.update(|cx| RunsHost::bind_execution(pane, run.exec(&id), cx));
    continue_in(pane, SESSION, cx);
    cx.update(|cx| {
        let exec = RunsHost::pane_execution(pane, cx).cloned().unwrap();
        seed::commit_now(
            vec![daruda_store::tasks::TaskEvent::Settled {
                exec,
                outcome: daruda_store::tasks::machine::SettleOutcome::Completed {
                    end_reason: daruda_store::tasks::SessionEndReason::Stop,
                },
                result: Some("tests fixed".into()),
                at: chrono::Utc::now(),
            }],
            cx,
        );
        let record = cx.global::<GlobalTasks>().get(&id).unwrap();
        assert!(matches!(record.lifecycle, Lifecycle::Done { .. }));
        assert_eq!(record.ended_executions.len(), 2);
        assert!(!record.ended_executions[0].is_continuation());
        assert_eq!(
            record.ended_executions[1].result.as_deref(),
            Some("tests fixed")
        );
    });
}
