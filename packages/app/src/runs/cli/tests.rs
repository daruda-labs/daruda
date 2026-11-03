mod wait_tests;
use super::*;
use crate::runs::seed::{self, SeedEnd, SeedRun};
use daruda_agent::hooks::status_file::StatusFile;
use daruda_store::tasks::{Lifecycle, SessionEndReason};
use gpui::TestAppContext;

fn terminal_run(cx: &mut gpui::App) -> (SeedRun, RunPane) {
    let terminal = crate::hooks::agent_hooks::mint_terminal_id();
    let mut run = SeedRun::cli("/cli", CliProcessState::Discovering);
    run.terminal_id = Some(terminal.as_str().to_owned());
    let pane = RunPane {
        workspace: WorkspaceUuid::new(),
        pane: 1,
    };
    RunsHost::register_terminal(pane, terminal, cx);
    (run, pane)
}

fn report_hook(pane: RunPane, session: &str, pid: Option<u32>, cx: &mut gpui::App) {
    let mut file = StatusFile::new_hook(
        session,
        "/cli",
        daruda_agent::SessionStatus::Idle,
        "SessionStart",
    );
    file.terminal_id = RunsHost::terminal_id(pane, cx).cloned();
    file.agent_pid = pid;
    RunsHost::record_task_cli_hook(&file, cx);
}

#[gpui::test]
fn a_hook_binds_a_run_without_claiming_an_unobserved_process(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::inline_commits_for_test(cx);
        let (run, pane) = terminal_run(cx);
        let id = seed::task(
            seed::spec(Default::default(), "CLI"),
            Some((&run, SeedEnd::Running)),
            cx,
        );
        RunsHost::bind_execution(pane, run.exec(&id), cx);
        report_hook(pane, "hook-session", None, cx);
        let task = cx.global::<GlobalTasks>().get(&id).unwrap();
        let execution = task.execution.as_ref().unwrap();
        assert_eq!(execution.session_id.as_deref(), Some("hook-session"));
        assert_eq!(
            execution.source.cli_process(),
            Some(&CliProcessState::Discovering)
        );
        assert!(
            task.session_ids
                .iter()
                .any(|session| session == "hook-session")
        );
    });
}

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
        let (run, pane) = terminal_run(cx);
        let id = seed::task(
            seed::spec(Default::default(), "CLI"),
            Some((&run, SeedEnd::Running)),
            cx,
        );
        RunsHost::bind_execution(pane, run.exec(&id), cx);
        report_hook(pane, "session", Some(42), cx);
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
        RunsHost::register_terminal(first, crate::hooks::agent_hooks::mint_terminal_id(), cx);
        RunsHost::register_terminal(second, crate::hooks::agent_hooks::mint_terminal_id(), cx);
        assert_ne!(
            RunsHost::terminal_id(first, cx),
            RunsHost::terminal_id(second, cx)
        );
        RunsHost::release_workspace(first.workspace, cx);
        assert!(RunsHost::terminal_id(first, cx).is_none());
        assert!(RunsHost::terminal_id(second, cx).is_some());
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

#[gpui::test]
fn releasing_a_replaced_view_keeps_its_successor_registered(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        let pane = RunPane {
            workspace: WorkspaceUuid::new(),
            pane: 1,
        };
        let old = crate::hooks::agent_hooks::mint_terminal_id();
        let fresh = crate::hooks::agent_hooks::mint_terminal_id();
        RunsHost::register_terminal(pane, old.clone(), cx);
        RunsHost::register_terminal(pane, fresh.clone(), cx);
        RunsHost::release_terminal(pane, &old, cx);
        assert_eq!(RunsHost::terminal_pane(&fresh, cx), Some(pane));
        assert!(RunsHost::terminal_pane(&old, cx).is_none());
        RunsHost::release_terminal(pane, &fresh, cx);
        assert!(RunsHost::terminal_pane(&fresh, cx).is_none());
    });
}

#[gpui::test]
fn a_stale_pane_binding_cannot_override_the_recorded_terminal(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::inline_commits_for_test(cx);
        let (run, pane) = terminal_run(cx);
        let id = seed::task(
            seed::spec(Default::default(), "CLI"),
            Some((&run, SeedEnd::Running)),
            cx,
        );
        let fresh = crate::hooks::agent_hooks::mint_terminal_id();
        RunsHost::register_terminal(pane, fresh, cx);
        RunsHost::bind_execution(pane, run.exec(&id), cx);
        report_hook(pane, "other-terminal-session", None, cx);
        assert!(
            cx.global::<GlobalTasks>()
                .get(&id)
                .unwrap()
                .execution
                .as_ref()
                .unwrap()
                .session_id
                .is_none()
        );
    });
}

#[gpui::test]
fn a_hook_older_than_the_run_cannot_attach_its_session(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::inline_commits_for_test(cx);
        let (run, pane) = terminal_run(cx);
        let id = seed::task(
            seed::spec(Default::default(), "CLI"),
            Some((&run, SeedEnd::Running)),
            cx,
        );
        RunsHost::bind_execution(pane, run.exec(&id), cx);
        let mut file = StatusFile::new_hook(
            "previous-session",
            "/cli",
            daruda_agent::SessionStatus::Idle,
            "SessionStart",
        );
        file.terminal_id = RunsHost::terminal_id(pane, cx).cloned();
        file.timestamp = chrono::DateTime::UNIX_EPOCH;
        RunsHost::record_task_cli_hook(&file, cx);
        assert!(
            cx.global::<GlobalTasks>()
                .get(&id)
                .unwrap()
                .execution
                .as_ref()
                .unwrap()
                .session_id
                .is_none()
        );
    });
}

#[gpui::test]
fn a_killed_cli_settles_even_when_its_terminal_remains_open(cx: &mut TestAppContext) {
    let child = test_process::sleeping();
    let pid = child.id();
    let (tracker, exits) = PtyTracker::spawn();
    let (id, pane) = cx.update(|cx| {
        RunsHost::inline_commits_for_test(cx);
        let (run, pane) = terminal_run(cx);
        let id = seed::task(
            seed::spec(Default::default(), "CLI"),
            Some((&run, SeedEnd::Running)),
            cx,
        );
        RunsHost::bind_execution(pane, run.exec(&id), cx);
        report_hook(pane, "killed-session", Some(pid), cx);
        tracker.track_agent("killed-session".into(), pid);
        (id, pane)
    });
    drop(child);
    tracker.poke();
    let exit = exits.recv_timeout(Duration::from_secs(5)).unwrap();
    cx.update(|cx| {
        RunsHost::apply_cli_tracker_event(exit, cx);
        let task = cx.global::<GlobalTasks>().get(&id).unwrap();
        assert!(matches!(
            task.lifecycle,
            Lifecycle::Error {
                cause: daruda_store::tasks::machine::ErrorCause::SessionFailed,
                ..
            }
        ));
        assert!(matches!(
            task.execution.as_ref().unwrap().source.cli_process(),
            Some(CliProcessState::ExitConfirmed { .. })
        ));
        assert_eq!(task.ended_executions.len(), 1);
        assert_eq!(
            task.execution
                .as_ref()
                .unwrap()
                .cli_session_end
                .as_ref()
                .unwrap()
                .reason,
            daruda_store::tasks::CliSessionEndReason::ProcessExited
        );
        assert!(RunsHost::terminal_id(pane, cx).is_some());
    });
}

#[gpui::test]
fn a_normal_exit_preserves_the_hook_completed_run(cx: &mut TestAppContext) {
    let mut child = daruda_core::process::command(test_process::executable())
        .args(["--sleep-ms", "100"])
        .spawn()
        .unwrap();
    let pid = child.id();
    let (tracker, exits) = PtyTracker::spawn();
    let id = cx.update(|cx| {
        RunsHost::inline_commits_for_test(cx);
        let (run, pane) = terminal_run(cx);
        let id = seed::task(
            seed::spec(Default::default(), "CLI"),
            Some((&run, SeedEnd::Running)),
            cx,
        );
        RunsHost::bind_execution(pane, run.exec(&id), cx);
        report_hook(pane, "normal-session", Some(pid), cx);
        tracker.track_agent("normal-session".into(), pid);
        RunsHost::settle_cli_session("normal-session", SessionEndReason::Stop, cx);
        id
    });
    cx.run_until_parked();
    assert!(child.wait().unwrap().success());
    tracker.poke();
    let exit = exits.recv_timeout(Duration::from_secs(5)).unwrap();
    cx.update(|cx| {
        RunsHost::apply_cli_tracker_event(exit, cx);
        let task = cx.global::<GlobalTasks>().get(&id).unwrap();
        assert!(matches!(task.lifecycle, Lifecycle::Done { .. }));
        assert_eq!(task.ended_executions.len(), 1);
        assert!(matches!(
            task.execution.as_ref().unwrap().source.cli_process(),
            Some(CliProcessState::ExitConfirmed { .. })
        ));
    });
}

#[gpui::test]
fn a_persisted_stop_wins_when_the_exit_arrives_before_the_watcher(cx: &mut TestAppContext) {
    for event in ["Stop", "SessionEnd", "StopFailure"] {
        cx.update(|cx| {
            RunsHost::inline_commits_for_test(cx);
            let (run, pane) = terminal_run(cx);
            let id = seed::task(
                seed::spec(Default::default(), "CLI"),
                Some((&run, SeedEnd::Running)),
                cx,
            );
            RunsHost::bind_execution(pane, run.exec(&id), cx);
            report_hook(pane, "unread-final-hook", Some(42), cx);
            let mut file = StatusFile::new_hook(
                "unread-final-hook",
                "/cli",
                daruda_agent::SessionStatus::Idle,
                event,
            );
            file.terminal_id = RunsHost::terminal_id(pane, cx).cloned();
            file.agent_pid = Some(42);
            let dir = daruda_agent::hooks::status_file::default_dir().unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            let path = daruda_agent::hooks::status_file::path_for(&dir, file.storage_key());
            daruda_agent::hooks::status_file::write_atomic(&path, &file).unwrap();
            RunsHost::record_cli_exit("unread-final-hook", 42, cx);
            let task = cx.global::<GlobalTasks>().get(&id).unwrap();
            if event == "StopFailure" {
                assert!(matches!(task.lifecycle, Lifecycle::Error { .. }));
            } else {
                assert!(
                    matches!(task.lifecycle, Lifecycle::Done { .. }),
                    "{event}: {:?}",
                    task.lifecycle
                );
            }
            assert_eq!(task.ended_executions.len(), 1);
            std::fs::remove_file(path).unwrap();
        });
    }
}

#[gpui::test]
fn ending_one_resumed_terminal_does_not_settle_another(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::inline_commits_for_test(cx);
        let mut ids = Vec::new();
        for pid in [41, 42] {
            let (run, pane) = terminal_run(cx);
            let id = seed::task(
                seed::spec(Default::default(), "CLI"),
                Some((&run, SeedEnd::Running)),
                cx,
            );
            RunsHost::bind_execution(pane, run.exec(&id), cx);
            report_hook(pane, "shared-session", Some(pid), cx);
            ids.push((id, run.terminal_id));
        }
        RunsHost::settle_cli_terminal(
            "shared-session",
            ids[0].1.clone(),
            SessionEndReason::Stop,
            cx,
        );
        assert!(matches!(
            cx.global::<GlobalTasks>().get(&ids[0].0).unwrap().lifecycle,
            Lifecycle::Done { .. }
        ));
        let second = cx.global::<GlobalTasks>().get(&ids[1].0).unwrap();
        assert!(matches!(second.lifecycle, Lifecycle::Running { .. }));
        assert_eq!(
            second.execution.as_ref().unwrap().source.cli_process(),
            Some(&CliProcessState::Running { pid: 42 })
        );
    });
}

#[gpui::test]
fn closing_the_original_cli_pane_preserves_the_deliberate_end_reason(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::inline_commits_for_test(cx);
        let (run, pane) = terminal_run(cx);
        let id = seed::task(
            seed::spec(Default::default(), "CLI"),
            Some((&run, SeedEnd::Running)),
            cx,
        );
        RunsHost::bind_execution(pane, run.exec(&id), cx);
        report_hook(pane, "closed-session", Some(42), cx);
        RunsHost::release_execution(
            pane,
            daruda_store::tasks::machine::UndeliveredReason::PaneClosed,
            cx,
        );
        RunsHost::record_cli_exit("closed-session", 42, cx);
        let task = cx.global::<GlobalTasks>().get(&id).unwrap();
        assert_eq!(
            task.execution
                .as_ref()
                .unwrap()
                .cli_session_end
                .as_ref()
                .unwrap()
                .reason,
            daruda_store::tasks::CliSessionEndReason::PaneClosed
        );
    });
}
