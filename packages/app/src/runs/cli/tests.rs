mod wait_tests;
use super::*;
use crate::runs::seed::{self, SeedEnd, SeedRun};
use daruda_agent::pty_tracker::PtyBinding;
use daruda_store::tasks::{Lifecycle, SessionEndReason};
use gpui::TestAppContext;

#[gpui::test]
fn cli_result_read_protects_the_execution_from_exit_reconciliation(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("result.jsonl");
    std::fs::write(
        &path,
        super::super::cli_result::tests::assistant("completed CLI answer", false),
    )
    .unwrap();
    let store = daruda_store::tasks::TaskStore::in_directory(
        dir.path().join("task-log"),
        None,
        "test",
        super::super::no_hook(),
    )
    .unwrap();
    let (id, settled_by) = cx.update(|cx| {
        RunsHost::install(RunsHost::open(Ok(store.clone())), cx);
        let run = SeedRun::cli("/cli", CliProcessState::Running { pid: 42 }).session("s");
        let id = seed::task(
            seed::spec(Default::default(), "CLI"),
            Some((&run, SeedEnd::Running)),
            cx,
        );
        seed::commit_now(
            vec![TaskEvent::CliTranscriptFound {
                exec: run.exec(&id),
                transcript_path: path.clone(),
                at: Utc::now(),
            }],
            cx,
        );
        RunsHost::settle_cli_session("s", SessionEndReason::Stop, cx);
        let settled_by = Utc::now();
        assert!(
            cx.global::<RunsHost>()
                .delivery
                .settling
                .contains(&run.exec(&id))
        );
        RunsHost::record_cli_exit("s", 42, cx);
        assert!(matches!(
            cx.global::<GlobalTasks>().get(&id).unwrap().lifecycle,
            Lifecycle::Running { .. }
        ));
        // The transcript read resolves later; its latency is not run time.
        std::thread::sleep(std::time::Duration::from_millis(20));
        (id, settled_by)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let task = cx.global::<GlobalTasks>().get(&id).unwrap();
        assert!(matches!(task.lifecycle, Lifecycle::Done { .. }));
        assert_eq!(
            task.ended_executions.last().unwrap().result.as_deref(),
            Some("completed CLI answer")
        );
        assert!(task.ended_executions.last().unwrap().finished_at <= settled_by);
        assert!(cx.global::<RunsHost>().delivery.settling.is_empty());
        assert_eq!(store.current().unwrap().1.get(&id), Some(task));
    });
}

#[gpui::test]
fn a_cli_result_read_cannot_settle_a_restarted_execution(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("answer.jsonl");
    std::fs::write(
        &path,
        super::super::cli_result::tests::assistant("old answer", false),
    )
    .unwrap();
    let (id, replacement) = cx.update(|cx| {
        RunsHost::inline_commits_for_test(cx);
        let run = SeedRun::cli("/cli", CliProcessState::Running { pid: 42 }).session("old");
        let id = seed::task(
            seed::spec(Default::default(), "CLI"),
            Some((&run, SeedEnd::Running)),
            cx,
        );
        RunsHost::record_cli_transcript("old", &path, cx);
        RunsHost::settle_cli_session("old", SessionEndReason::Stop, cx);
        seed::commit_now(
            vec![
                TaskEvent::CancelRequested {
                    exec: run.exec(&id),
                    at: Utc::now(),
                },
                TaskEvent::Reopened {
                    id: id.clone(),
                    run_in: Default::default(),
                    at: Utc::now(),
                },
            ],
            cx,
        );
        let replacement = SeedRun::cli("/cli", CliProcessState::Running { pid: 43 });
        seed::commit_now(seed::run(&id, &replacement, SeedEnd::Running), cx);
        (id, replacement)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let record = cx.global::<GlobalTasks>().get(&id).unwrap();
        assert!(matches!(&record.lifecycle, Lifecycle::Running { exec, .. } if exec == &replacement.exec(&id)));
        assert_eq!(record.ended_executions.len(), 1);
        assert_eq!(record.ended_executions[0].result, None);
        assert!(cx.global::<RunsHost>().delivery.settling.is_empty());
    });
}

#[gpui::test]
fn an_exit_after_a_pending_binding_is_recorded_durably(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let store = daruda_store::tasks::TaskStore::in_directory(
        directory.path(),
        None,
        "test",
        super::super::no_hook(),
    )
    .unwrap();
    let id = cx.update(|cx| {
        RunsHost::install(RunsHost::open(Ok(store.clone())), cx);
        let run = SeedRun::cli("/cli", CliProcessState::Discovering);
        let id = seed::task(
            seed::spec(Default::default(), "CLI"),
            Some((&run, SeedEnd::Running)),
            cx,
        );
        let pane = RunPane {
            workspace: WorkspaceUuid::new(),
            pane: 1,
        };
        RunsHost::bind_execution(pane, run.exec(&id), cx);
        RunsHost::record_cli_binding(
            Some(pane),
            &PtyBinding {
                claude_pid: 42,
                session_id: "session".into(),
            },
            None,
            cx,
        );
        RunsHost::record_cli_exit("session", 42, cx);
        id
    });
    cx.run_until_parked();
    let durable = store.current().unwrap().1;
    let process = durable
        .get(&id)
        .unwrap()
        .execution
        .as_ref()
        .unwrap()
        .source
        .cli_process();
    assert!(
        matches!(process, Some(CliProcessState::ExitConfirmed { .. })),
        "{process:?}"
    );
    cx.update(|cx| assert_eq!(cx.global::<GlobalTasks>().get(&id), durable.get(&id)));
}

#[gpui::test]
fn the_tracker_keeps_both_windows_panes_distinct(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        let first = RunPane {
            workspace: WorkspaceUuid::new(),
            pane: 1,
        };
        let second = RunPane {
            workspace: WorkspaceUuid::new(),
            pane: 1,
        };
        RunsHost::register_terminal(first, 4242, cx);
        RunsHost::register_terminal(second, 4243, cx);
        assert_ne!(
            RunsHost::tracked_terminal(first, cx),
            RunsHost::tracked_terminal(second, cx)
        );
        RunsHost::release_workspace(first.workspace, cx);
        assert!(RunsHost::tracked_terminal(first, cx).is_none());
        assert!(RunsHost::tracked_terminal(second, cx).is_some());
    });
}

#[gpui::test]
fn a_restored_cli_without_a_live_process_does_not_stay_running(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::inline_commits_for_test(cx);
        let run = SeedRun::cli("/cli", CliProcessState::Running { pid: 42 }).session("restored");
        let id = seed::task(
            seed::spec(Default::default(), "CLI"),
            Some((&run, SeedEnd::Running)),
            cx,
        );
        RunsHost::record_cli_exit("restored", 42, cx);
        assert!(matches!(
            cx.global::<GlobalTasks>().get(&id).unwrap().lifecycle,
            Lifecycle::Interrupted {
                cause: daruda_store::tasks::machine::InterruptCause::AppRestart,
                ..
            }
        ));
        let seq = RunsHost::sequence_for_test(cx);
        RunsHost::record_cli_exit("restored", 42, cx);
        assert_eq!(RunsHost::sequence_for_test(cx), seq);
    });
}
