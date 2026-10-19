use super::*;
use crate::runs::seed::{self, SeedEnd, SeedRun};
use daruda_store::tasks::{TaskEvent, TaskRunIn};
use gpui::TestAppContext;

#[gpui::test]
fn exact_results_survive_history_eviction_and_latest_resolves_after_reopen(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        let run = SeedRun::chat("/work");
        let id = seed::task(
            seed::spec(Default::default(), "task"),
            Some((&run, SeedEnd::Done)),
            cx,
        );
        let first = RunKey::Task(run.exec(&id));
        RunsHost::register_run(first.clone(), cx);
        let original = RunsHost::run_query(first.clone(), cx).read();
        for index in 0..6 {
            RunsHost::commit_blocking(
                TaskEvent::Reopened {
                    id: id.clone(),
                    run_in: TaskRunIn::default(),
                    at: chrono::Utc::now(),
                },
                cx,
            )
            .unwrap();
            RunsHost::commit_blocking(
                TaskEvent::Edited {
                    id: id.clone(),
                    at: chrono::Utc::now(),
                    edit: daruda_store::tasks::machine::TaskEdit {
                        title: format!("edited {index}"),
                        prompt: format!("prompt {index}"),
                        notes: String::new(),
                        auto_execute: true,
                        agent_surface: Default::default(),
                        base_worktree_path: None,
                        branch: None,
                        run_in: None,
                        project: None,
                    },
                },
                cx,
            )
            .unwrap();
            let run = SeedRun::chat("/work");
            seed::commit_now(seed::run(&id, &run, SeedEnd::Done), cx);
        }
        let last = RunsHost::latest_task_key(&id, cx).unwrap();
        assert_ne!(last, first);
        RunsHost::commit_blocking(
            TaskEvent::Reopened {
                id: id.clone(),
                run_in: TaskRunIn::default(),
                at: chrono::Utc::now(),
            },
            cx,
        )
        .unwrap();
        assert_eq!(RunsHost::latest_task_key(&id, cx), Some(last));
        assert!(matches!(
            RunsHost::run_query(first.clone(), cx).read().state,
            RunState::Ended { .. }
        ));
        assert_eq!(RunsHost::run_query(first.clone(), cx).read(), original);
        RunsHost::seed_for_test(cx);
        assert_eq!(RunsHost::run_scope(cx), vec![first.clone()]);
        assert!(matches!(
            RunsHost::run_query(first, cx).read().state,
            RunState::Ended { .. }
        ));
    });
}

#[gpui::test]
fn explicit_unknown_flow_and_missing_task_have_distinct_states(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        assert!(matches!(
            RunsHost::run_query(RunKey::Flow("absent".into()), cx)
                .read()
                .state,
            RunState::Unknown
        ));
        let task = RunKey::Task(daruda_store::tasks::ExecutionRef {
            task_id: "absent".into(),
            execution_id: "exec".into(),
        });
        assert!(matches!(
            RunsHost::run_query(task, cx).read().state,
            RunState::Gone
        ));
        assert!(RunsHost::run_scope(cx).is_empty());
    });
}

#[gpui::test]
fn held_flow_end_wins_over_disk_corruption_and_survives_directory_removal(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    std::fs::create_dir(&run_dir).unwrap();
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        let view = crate::runs::RunView::new(run_dir.clone(),crate::runs::FlowSource::Resumed {run_dir:run_dir.clone()},vec![]);
        RunsHost::seed_flow_run(dir.path().to_path_buf(),view,cx);
        let key = RunKey::Flow("run".into());
        assert!(matches!(RunsHost::run_query(key.clone(),cx).read().state,RunState::Running {..}));
        RunsHost::apply_flow_event_for_test(dir.path(), &daruda_flow::event::FlowEvent::RunEnded {end:daruda_flow::event::RunEnd::NotResumed {message:"frozen failure".into()}},cx);
        std::fs::remove_dir(&run_dir).unwrap();
        assert!(matches!(RunsHost::run_query(key,cx).read().state,RunState::Ended {text:Some(text),..} if text == "frozen failure"));
    });
}

#[gpui::test]
fn startup_carries_only_previously_active_orchestrator_executions(cx: &mut TestAppContext) {
    use daruda_store::tasks::{CliProcessState, TaskStore};
    let dir = tempfile::tempdir().unwrap();
    let store = TaskStore::in_directory(dir.path(), None, "test", crate::runs::no_hook()).unwrap();
    let mut carried = Vec::new();
    for (source, end, origin) in [
        (
            false,
            SeedEnd::Running,
            Some(daruda_store::tasks::StartedBy::Orchestrator),
        ),
        (
            true,
            SeedEnd::Running,
            Some(daruda_store::tasks::StartedBy::Orchestrator),
        ),
        (
            false,
            SeedEnd::StartFailed,
            Some(daruda_store::tasks::StartedBy::Orchestrator),
        ),
        (
            false,
            SeedEnd::Running,
            Some(daruda_store::tasks::StartedBy::Desktop),
        ),
    ] {
        let spec = seed::spec(Default::default(), "task");
        let run = if source {
            SeedRun::cli("/work", CliProcessState::Discovering)
        } else {
            SeedRun::chat("/work")
        };
        if origin == Some(daruda_store::tasks::StartedBy::Orchestrator) && end == SeedEnd::Running {
            carried.push(RunKey::Task(run.exec(&spec.id)));
        }
        let mut events = seed::created(spec.clone());
        events.extend(seed::run(&spec.id, &run, end));
        for mut event in events {
            if let TaskEvent::DispatchRequested { started_by, .. } = &mut event {
                *started_by = origin;
            }
            smol::block_on(store.propose(event)).unwrap();
        }
    }
    cx.update(|cx| {
        RunsHost::install(RunsHost::open(Ok(store)), cx);
        let scope = RunsHost::run_scope(cx);
        assert_eq!(scope.len(), 2);
        for key in carried {
            assert!(scope.contains(&key));
        }
    });
}

#[gpui::test]
fn flow_question_level_is_visible_and_clears_when_stopped(cx: &mut TestAppContext) {
    let tree = std::path::PathBuf::from("/work/blocked-flow");
    let run_dir = tree.join("run");
    let (reply, _rx) = smol::channel::bounded(1);
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        let view = crate::runs::RunView::new(run_dir.clone(), crate::runs::FlowSource::Resumed {run_dir:run_dir.clone()}, vec![]);
        RunsHost::seed_flow_run(tree.clone(),view,cx);
        RunsHost::park_flow_ask(&tree,&run_dir,daruda_flow::runner::PendingAsk {
            owner:daruda_flow::runner::AskOwner::Node {node:"asker".into(),generation:0,attempt:1}, ask_id:1,
            request:daruda_flow::runner::AskRequest {tool:"write".into(),detail:None,options:vec![]},reply,
        },cx);
        let key=RunKey::Flow("run".into());
        assert!(matches!(RunsHost::run_query(key.clone(),cx).read().state,RunState::Running {waiting_on:Some(super::super::result::WaitingOn::Question {node}),..} if node=="asker"));
        RunsHost::stop_flow(&tree,cx);
        assert!(matches!(RunsHost::run_query(key,cx).read().state,RunState::Running {waiting_on:None,..}));
    });
}

#[gpui::test]
fn permission_levels_apply_only_to_the_exact_running_chat_execution(cx: &mut TestAppContext) {
    use crate::runs::result::WaitingOn;
    use daruda_store::tasks::CliProcessState;
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        let chat = SeedRun::chat("/work");
        let id = seed::task(
            seed::spec(Default::default(), "chat"),
            Some((&chat, SeedEnd::Running)),
            cx,
        );
        let key = RunKey::Task(chat.exec(&id));
        let waiting = Some(WaitingOn::Permission { tool: None });
        RunsHost::observe_task_waiting(key.clone(), waiting.clone(), cx);
        assert!(matches!(
            RunsHost::run_query(key.clone(), cx).read().state,
            RunState::Running {
                waiting_on: Some(WaitingOn::Permission { .. }),
                ..
            }
        ));
        RunsHost::observe_task_waiting(key.clone(), None, cx);
        assert!(matches!(
            RunsHost::run_query(key, cx).read().state,
            RunState::Running {
                waiting_on: None,
                ..
            }
        ));
        let cli = SeedRun::cli("/work", CliProcessState::Discovering);
        let id = seed::task(
            seed::spec(Default::default(), "cli"),
            Some((&cli, SeedEnd::Running)),
            cx,
        );
        let key = RunKey::Task(cli.exec(&id));
        RunsHost::observe_task_waiting(key.clone(), waiting.clone(), cx);
        assert!(matches!(
            RunsHost::run_query(key, cx).read().state,
            RunState::Running {
                waiting_on: None,
                ..
            }
        ));
        for end in [SeedEnd::Preparing, SeedEnd::Dispatching] {
            let run = SeedRun::chat("/work");
            let id = seed::task(
                seed::spec(Default::default(), "not running"),
                Some((&run, end)),
                cx,
            );
            let key = RunKey::Task(run.exec(&id));
            RunsHost::observe_task_waiting(key.clone(), waiting.clone(), cx);
            assert!(matches!(
                RunsHost::run_query(key, cx).read().state,
                RunState::Running {
                    waiting_on: None,
                    ..
                }
            ));
        }
        let absent = RunKey::Task(chat.exec("missing"));
        RunsHost::observe_task_waiting(absent.clone(), waiting, cx);
        assert!(matches!(
            RunsHost::run_query(absent, cx).read().state,
            RunState::Gone
        ));
    });
}

#[gpui::test]
fn flow_answer_and_closed_reply_clear_the_waiting_level(cx: &mut TestAppContext) {
    let tree = std::path::PathBuf::from("/work/answered-flow");
    let run_dir = tree.join("run");
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        RunsHost::seed_flow_run(
            tree.clone(),
            crate::runs::RunView::new(
                run_dir.clone(),
                crate::runs::FlowSource::Resumed {
                    run_dir: run_dir.clone(),
                },
                vec![],
            ),
            cx,
        );
        let key = RunKey::Flow("run".into());
        RunsHost::register_run(key.clone(), cx);
        for id in 1..=2 {
            let (reply, rx) = smol::channel::bounded(1);
            RunsHost::park_flow_ask(
                &tree,
                &run_dir,
                daruda_flow::runner::PendingAsk {
                    owner: daruda_flow::runner::AskOwner::Node {
                        node: "asker".into(),
                        generation: 0,
                        attempt: 1,
                    },
                    ask_id: id,
                    request: daruda_flow::runner::AskRequest {
                        tool: "write".into(),
                        detail: None,
                        options: vec![],
                    },
                    reply,
                },
                cx,
            );
            assert!(matches!(
                RunsHost::run_query(key.clone(), cx).read().state,
                RunState::Running {
                    waiting_on: Some(_),
                    ..
                }
            ));
            if id == 1 {
                RunsHost::answer_flow_ask(&tree, id, daruda_acp::PermissionDecision::Cancelled, cx)
                    .unwrap();
            } else {
                drop(rx);
                RunsHost::apply_flow_event_for_test(
                    &tree,
                    &daruda_flow::event::FlowEvent::NodeStarted {
                        node: "other".into(),
                        attempt: 1,
                    },
                    cx,
                );
            }
            assert!(matches!(
                RunsHost::run_query(key.clone(), cx).read().state,
                RunState::Running {
                    waiting_on: None,
                    ..
                }
            ));
        }
    });
}

#[gpui::test]
fn a_late_question_cannot_block_a_stopped_flow(cx: &mut TestAppContext) {
    let tree = std::path::PathBuf::from("/work/stopped-flow");
    let run_dir = tree.join("run");
    let (reply, _rx) = smol::channel::bounded(1);
    cx.update(|cx| {
        RunsHost::ensure_installed(cx);
        RunsHost::seed_flow_run(
            tree.clone(),
            crate::runs::RunView::new(
                run_dir.clone(),
                crate::runs::FlowSource::Resumed {
                    run_dir: run_dir.clone(),
                },
                vec![],
            ),
            cx,
        );
        RunsHost::stop_flow(&tree, cx);
        RunsHost::park_flow_ask(
            &tree,
            &run_dir,
            daruda_flow::runner::PendingAsk {
                owner: daruda_flow::runner::AskOwner::Node {
                    node: "asker".into(),
                    generation: 0,
                    attempt: 1,
                },
                ask_id: 1,
                request: daruda_flow::runner::AskRequest {
                    tool: "write".into(),
                    detail: None,
                    options: vec![],
                },
                reply,
            },
            cx,
        );
        assert!(matches!(
            RunsHost::run_query(RunKey::Flow("run".into()), cx)
                .read()
                .state,
            RunState::Running {
                waiting_on: None,
                ..
            }
        ));
    });
}

mod episodes;
