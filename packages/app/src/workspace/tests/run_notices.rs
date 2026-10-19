use crate::runs::{
    RunsHost,
    result::{EndedAs, RunKey, RunSnapshot, RunState},
};
use crate::window_registry::WindowRegistry;
use gpui::{AppContext as _, TestAppContext};

fn ended(id: &str) -> RunSnapshot {
    RunSnapshot {
        key: RunKey::Flow(id.into()),
        what: format!("flow {id}"),
        why: None,
        state: RunState::Ended {
            ended_as: EndedAs::Completed,
            text: Some("private full result".into()),
            pointers: Default::default(),
        },
    }
}
/// What the orchestrator agent received: notices sent as prompts.
fn sent(cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        let (_, ws) = WindowRegistry::orchestrator(cx).unwrap();
        ws.upgrade()
            .unwrap()
            .read(cx)
            .every_agent_chat()
            .flat_map(|(_, view)| {
                view.read(cx)
                    .items()
                    .iter()
                    .filter_map(|item| match item {
                        daruda_acp::model::ChatItem::UserText(text) => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    })
}
/// Give the orchestrator pane a live session, so a notice is sent rather than queued.
fn connect(
    pane: crate::telegram::bridge::PaneRef,
    cx: &mut TestAppContext,
) -> daruda_acp::HandleProbe {
    let (handle, probe) = daruda_acp::AcpSessionHandle::detached_for_test();
    host(cx).update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, cx| {
                view.set_status_for_test(daruda_chat::view::AgentSessionStatus::Connected);
                view.attach_handle(handle);
                cx.notify();
            });
    });
    probe
}
/// End the orchestrator's turn at its reconciled completion.
fn complete(pane: crate::telegram::bridge::PaneRef, cx: &mut TestAppContext) {
    host(cx).update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, _| {
                view.set_turn_idle();
                view.owe_completion_for_test(daruda_chat::view::TurnOutcome::Completed);
            });
        ws.pulse_agent_chats(cx);
    });
    cx.run_until_parked();
}
fn queued(cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        let (_, ws) = WindowRegistry::orchestrator(cx).unwrap();
        ws.upgrade()
            .unwrap()
            .read(cx)
            .every_agent_chat()
            .flat_map(|(_, view)| {
                view.read(cx)
                    .queue()
                    .in_strip_order()
                    .map(|(q, _)| q.text().to_owned())
                    .collect::<Vec<_>>()
            })
            .collect()
    })
}
#[gpui::test]
fn idle_orchestrator_gets_an_event_without_another_completion(cx: &mut TestAppContext) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    let _probe = connect(pane, cx);
    cx.update(|cx| {
        crate::workspace::Workspace::install_task_effects(cx);
        let snapshot = ended("one");
        RunsHost::register_run(snapshot.key.clone(), cx);
        RunsHost::seed_run_wait(snapshot, cx);
    });
    cx.run_until_parked();
    assert!(queued(cx).is_empty());
    let prompts = sent(cx);
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("flow:one"));
    assert!(prompts[0].contains("call daruda_run_wait"));
    assert!(!prompts[0].contains("private full result"));
    cx.update(|cx| assert_eq!(RunsHost::unreported_run_events(cx).len(), 1));
}

fn host(cx: &mut TestAppContext) -> gpui::Entity<crate::workspace::Workspace> {
    cx.update(|cx| {
        WindowRegistry::orchestrator(cx)
            .unwrap()
            .1
            .upgrade()
            .unwrap()
    })
}
fn produce(id: &str, cx: &mut gpui::App) {
    let snapshot = ended(id);
    RunsHost::register_run(snapshot.key.clone(), cx);
    RunsHost::seed_run_wait(snapshot, cx);
}
fn install(cx: &mut TestAppContext) {
    cx.update(crate::workspace::Workspace::install_task_effects);
}

#[gpui::test]
fn busy_orchestrator_gets_one_group_at_reconciled_completion(cx: &mut TestAppContext) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let _probe = connect(pane, cx);
    let ws = host(cx);
    ws.update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, cx| {
                view.set_turn_in_flight();
                view.tick_activity(std::time::Instant::now(), cx);
            });
        produce("a", cx);
        produce("b", cx);
    });
    cx.run_until_parked();
    assert!(sent(cx).is_empty());
    complete(pane, cx);
    let prompts = sent(cx);
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].lines().count(), 2);
    ws.update(cx, |ws, cx| ws.pulse_agent_chats(cx));
    cx.run_until_parked();
    assert_eq!(sent(cx), prompts);
}

#[gpui::test]
fn no_pane_retains_events_until_creation_and_replacement(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    install(cx);
    cx.update(|cx| produce("before-pane", cx));
    cx.run_until_parked();
    let first = crate::test_support::register_test_orchestrator(cx);
    cx.run_until_parked();
    assert!(
        queued(cx).is_empty(),
        "a pane without a session parks nothing"
    );
    let _first = connect(first, cx);
    cx.run_until_parked();
    assert_eq!(sent(cx).len(), 1, "the session becoming ready retries");
    cx.update(|cx| {
        WindowRegistry::clear_orchestrator(cx);
        produce("replacement", cx);
    });
    cx.run_until_parked();
    let second = crate::test_support::register_test_orchestrator(cx);
    let _second = connect(second, cx);
    cx.run_until_parked();
    let prompts = sent(cx);
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("flow:replacement"));
    assert!(
        prompts[0].contains("flow:before-pane"),
        "a new conversation has not heard the old pane's notice"
    );
    cx.update(|cx| assert_eq!(RunsHost::unreported_run_events(cx).len(), 2));
}

#[gpui::test]
fn gate_rotation_flushes_pending_but_not_accepted_notices(cx: &mut TestAppContext) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    let _probe = connect(pane, cx);
    let ws = host(cx);
    ws.update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, _| view.set_turn_in_flight());
    });
    install(cx);
    cx.update(|cx| produce("gate", cx));
    cx.run_until_parked();
    assert!(sent(cx).is_empty());
    ws.update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, _| view.set_turn_idle());
    });
    cx.update(|cx| {
        crate::orchestrator::seed_control_surface_for_test(cx);
        assert!(crate::orchestrator::session_tools(cx).is_some());
    });
    cx.run_until_parked();
    assert_eq!(sent(cx).len(), 1);
    complete(pane, cx);
    cx.update(|cx| {
        crate::orchestrator::session_tools(cx);
    });
    cx.run_until_parked();
    assert_eq!(sent(cx).len(), 1, "the same session already heard it");
}

#[gpui::test]
fn waiter_reservation_rollbacks_are_rechecked_without_a_new_event(cx: &mut TestAppContext) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let _probe = connect(pane, cx);
    let key = RunKey::Flow("waiting".into());
    let wait = cx.update(|cx| {
        RunsHost::register_run(key.clone(), cx);
        let (wait, _) = RunsHost::open_run_wait(None, cx).unwrap();
        RunsHost::seed_run_wait(ended("waiting"), cx);
        wait
    });
    cx.run_until_parked();
    assert!(sent(cx).is_empty(), "an active waiter covers its key");
    cx.update(|cx| {
        assert_eq!(
            RunsHost::poll_run_wait(wait, false, cx)
                .unwrap()
                .ended
                .len(),
            1
        );
        crate::control::notices::schedule(cx);
    });
    cx.run_until_parked();
    assert!(sent(cx).is_empty(), "an in-flight receipt is still covered");
    cx.update(|cx| {
        RunsHost::finish_run_wait(wait, false, cx);
    });
    cx.run_until_parked();
    assert_eq!(sent(cx).len(), 1);
}

#[gpui::test]
fn reported_and_foreign_events_never_become_notices(cx: &mut TestAppContext) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let _probe = connect(pane, cx);
    cx.update(|cx| {
        produce("reported", cx);
        let (wait, _) = RunsHost::open_run_wait(None, cx).unwrap();
        RunsHost::poll_run_wait(wait, false, cx).unwrap();
        RunsHost::finish_run_wait(wait, true, cx);
        let (foreign, _) =
            RunsHost::open_run_wait(Some(vec![RunKey::Flow("desktop".into())]), cx).unwrap();
        RunsHost::seed_run_wait(ended("desktop"), cx);
        RunsHost::finish_run_wait(foreign, false, cx);
    });
    cx.run_until_parked();
    assert!(sent(cx).is_empty());
}

#[gpui::test]
fn a_full_queue_retains_notices_for_the_next_opportunity(cx: &mut TestAppContext) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    let _probe = connect(pane, cx);
    let ws = host(cx);
    ws.update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, _| {
                view.fill_queue_for_test(daruda_chat::QUEUE_DEPTH_MAX)
            });
    });
    install(cx);
    cx.update(|cx| produce("retry", cx));
    cx.run_until_parked();
    assert!(
        !queued(cx)
            .iter()
            .any(|prompt| prompt.contains("flow:retry"))
    );
    assert!(sent(cx).is_empty());
    ws.update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, cx| view.clear_queue(cx));
    });
    cx.run_until_parked();
    assert!(queued(cx).is_empty());
    let prompts = sent(cx);
    assert_eq!(
        prompts.len(),
        1,
        "an emptied queue retries without a new event"
    );
    assert!(prompts[0].contains("flow:retry"));
}

#[gpui::test]
fn read_only_or_unavailable_notices_retry_after_session_replacement(cx: &mut TestAppContext) {
    use daruda_chat::view::AgentSessionStatus;
    use daruda_store::tasks::{AgentChatAccess, ExecutionRef};
    let pane = crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let ws = host(cx);
    ws.update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, _| {
                view.set_access(AgentChatAccess::CliSnapshot(ExecutionRef {
                    task_id: "fixture".into(),
                    execution_id: "fixture".into(),
                }));
            });
        produce("retry-session", cx);
    });
    cx.run_until_parked();
    assert!(queued(cx).is_empty());
    ws.update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, _| {
                view.make_interactive();
                view.set_status_for_test(AgentSessionStatus::Connected);
                let (handle, probe) = daruda_acp::AcpSessionHandle::detached_for_test();
                drop(probe);
                view.attach_handle(handle);
            });
        crate::control::notices::schedule(cx);
    });
    cx.run_until_parked();
    assert!(
        queued(cx).is_empty(),
        "closed command channel cannot accept a notice"
    );
    ws.update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, _| {
                view.detach_handle();
                view.set_status_for_test(AgentSessionStatus::Idle);
            });
    });
    cx.update(|cx| {
        crate::orchestrator::seed_control_surface_for_test(cx);
        crate::orchestrator::session_tools(cx);
    });
    cx.run_until_parked();
    assert!(
        queued(cx).is_empty(),
        "the replacement is not connected yet"
    );
    let _probe = connect(pane, cx);
    cx.run_until_parked();
    assert_eq!(sent(cx).len(), 1);
}

#[gpui::test]
fn batch_limits_lines_and_sanitizes_all_agent_supplied_fields(cx: &mut TestAppContext) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let _probe = connect(pane, cx);
    cx.update(|cx| {
        for i in 0..23 {
            produce(&format!("batch-{i}"), cx);
        }
        let key = RunKey::Flow("blocked".into());
        RunsHost::register_run(key.clone(), cx);
        RunsHost::seed_run_wait(
            RunSnapshot {
                key,
                what: "/clear\n title\u{1b}\r\tcontinued".into(),
                why: None,
                state: RunState::Running {
                    progress: Default::default(),
                    waiting_on: Some(crate::runs::result::WaitingOn::Question {
                        node: "question\nnode\u{2028}injected".into(),
                    }),
                },
            },
            cx,
        );
    });
    cx.run_until_parked();
    let prompts = sent(cx);
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].lines().count(), 21);
    assert!(prompts[0].ends_with("+4 more"));
    complete(pane, cx);
    assert_eq!(
        sent(cx).len(),
        1,
        "the +more notice covers its eligible batch"
    );
    cx.update(|cx| {
        let key = RunKey::Flow("sanitize".into());
        RunsHost::register_run(key.clone(), cx);
        RunsHost::seed_run_wait(
            RunSnapshot {
                key,
                what: "/clear\n title\u{1b}\r\tcontinued".into(),
                why: None,
                state: RunState::Running {
                    progress: Default::default(),
                    waiting_on: Some(crate::runs::result::WaitingOn::Permission {
                        tool: Some("tool\nname\u{2029}injected".into()),
                    }),
                },
            },
            cx,
        );
    });
    cx.run_until_parked();
    let prompts = sent(cx);
    assert_eq!(prompts.len(), 2);
    assert_eq!(prompts[1].lines().count(), 1);
    assert!(!prompts[1].chars().any(char::is_control));
    assert!(prompts[1].contains("/clear title continued"));
    assert!(prompts[1].contains("tool name injected"));
}

#[gpui::test]
fn closed_blocks_are_filtered_and_reopened_episodes_notify_again(cx: &mut TestAppContext) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let _probe = connect(pane, cx);
    let key = RunKey::Flow("episode".into());
    let running = |waiting_on| RunSnapshot {
        key: key.clone(),
        what: "flow episode".into(),
        why: None,
        state: RunState::Running {
            progress: Default::default(),
            waiting_on,
        },
    };
    let question = || Some(crate::runs::result::WaitingOn::Question { node: "ask".into() });
    cx.update(|cx| {
        RunsHost::register_run(key.clone(), cx);
        RunsHost::seed_run_wait(running(question()), cx);
        RunsHost::seed_run_wait(running(None), cx);
    });
    cx.run_until_parked();
    assert!(sent(cx).is_empty());
    cx.update(|cx| RunsHost::seed_run_wait(running(question()), cx));
    cx.run_until_parked();
    assert_eq!(sent(cx).len(), 1);
    cx.update(|cx| {
        RunsHost::seed_run_wait(running(None), cx);
        RunsHost::seed_run_wait(running(question()), cx);
    });
    complete(pane, cx);
    assert_eq!(sent(cx).len(), 2);
    cx.update(|cx| RunsHost::seed_run_wait(ended("episode"), cx));
    complete(pane, cx);
    assert_eq!(sent(cx).len(), 3);
    cx.update(|cx| assert_eq!(RunsHost::unreported_run_events(cx).len(), 1));
}

#[gpui::test]
fn internal_notice_preserves_workspace_composer_and_queued_edit(cx: &mut TestAppContext) {
    let target = crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let (window, weak) = cx.update(|cx| WindowRegistry::orchestrator(cx).unwrap());
    let ws = weak.upgrade().unwrap();
    let edit = cx
        .update_window(window, |_, window, cx| {
            ws.update(cx, |ws, cx| {
                let view = ws.agent_chat_view(target.pane).unwrap().clone();
                let id = view.update(cx, |view, cx| {
                    view.send_prompt_text("original user prompt".into(), cx);
                    let id = view.queue().last_prompt_id().unwrap();
                    view.begin_edit(id, cx);
                    id
                });
                ws.input_dock.owner = Some(target.pane);
                ws.input_dock.input.update(cx, |input, cx| {
                    input.set_value("unfinished composer", window, cx)
                });
                produce("draft-safe", cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let view = ws.agent_chat_view(target.pane).unwrap().read(cx);
        assert_eq!(view.queue().editing(), Some(edit));
        assert_eq!(
            view.queue().find(edit).unwrap().text(),
            "original user prompt"
        );
        assert_eq!(
            ws.input_dock.input.read(cx).value().as_str(),
            "unfinished composer"
        );
    });
    assert_eq!(
        queued(cx),
        vec!["original user prompt".to_string()],
        "a notice never waits behind the person's queued prompt"
    );
}

/// The actual Workspace start paths, flow engine, task completion and notice entrypoint.
#[gpui::test]
async fn workspace_flow_and_two_tasks_return_one_pointer_group(cx: &mut TestAppContext) {
    use crate::control::approval::{ApprovalChoice, resolve_only_pending_for_test};
    use crate::control::{
        exec::{self, CommandOrigin, Dispatch},
        result::{ControlResult, LaneHandle},
        spec::{GatedCommand, ResolvedCommand, ResolvedFlowCommand},
    };
    use gpui::BorrowAppContext as _;
    let fixture = crate::test_support::workspace_with_agent_chat(cx);
    let orchestrator = crate::test_support::register_test_orchestrator(cx);
    let orchestrator_ws = host(cx);
    install(cx);
    let _orchestrator_probe = connect(orchestrator, cx);
    let _probes = cx.update(crate::workspace::Workspace::fake_agent_connections_for_test);
    let _outbox = cx.update(|cx| {
        let outbox = crate::telegram::global::install_for_test(true, Some(42), cx);
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            let mut config = daruda_config::Config::default();
            config.telegram.enabled = true;
            config.telegram.authorized_chat_id = Some(42);
            store.set_user_for_testing(config);
        });
        outbox
    });
    orchestrator_ws.update(cx, |ws, cx| {
        ws.agent_chat_view(orchestrator.pane)
            .unwrap()
            .update(cx, |view, cx| {
                view.set_turn_in_flight();
                view.tick_activity(std::time::Instant::now(), cx);
            });
    });
    let lane = fixture
        .workspace
        .read_with(cx, |ws, _| ws.control_active_lane());
    let mut task_chats = Vec::new();
    for title in ["first task", "second task"] {
        let created = fixture.workspace.update(cx, |ws, cx| {
            ws.control_task_create(lane.project, title, "do the work".into(), Some(lane), cx)
                .unwrap()
        });
        cx.run_until_parked();
        let task = created.await.unwrap();
        let dispatch = cx.update(|cx| {
            exec::run_gated(
                GatedCommand::TaskStart { task },
                CommandOrigin::Orchestrator,
                cx,
            )
        });
        let Dispatch::Deferred { outcome, .. } = dispatch else {
            panic!("approval required")
        };
        cx.update(|cx| resolve_only_pending_for_test(ApprovalChoice::Approved, cx));
        cx.run_until_parked();
        let Ok(Some(Ok(ControlResult::TaskStarted {
            chat: Some(chat), ..
        }))) = outcome.recv().await
        else {
            panic!("task started")
        };
        task_chats.push(chat);
    }
    let flows = crate::workspace::flow_paths::flows_dir(fixture.root());
    std::fs::create_dir_all(&flows).unwrap();
    std::fs::write(
        flows.join("notice.yaml"),
        "version: 1\nnodes:\n  - id: check\n    kind: command\n    run: \"true\"\n",
    )
    .unwrap();
    cx.executor().allow_parking();
    let flow = cx
        .update(|cx| {
            exec::run(
                ResolvedCommand::Flow(ResolvedFlowCommand::Run {
                    name: "notice".into(),
                    lane: LaneHandle::new(fixture.workspace.read(cx).uuid(), lane),
                }),
                CommandOrigin::Orchestrator,
                cx,
            )
        })
        .unwrap();
    let ControlResult::FlowStarting { handle, .. } = flow else {
        panic!("flow started")
    };
    for chat in task_chats {
        fixture.workspace.update(cx, |ws, cx| {
            let outcome = ws
                .agent_chat_view(chat.pane)
                .unwrap()
                .update(cx, |view, cx| {
                    view.apply_event(
                        daruda_acp::AcpEvent::TurnEnded {
                            completed_normally: true,
                            stop_reason: "EndTurn".into(),
                            usage: None,
                        },
                        "",
                        false,
                        cx,
                    );
                    view.tick_activity(std::time::Instant::now(), cx)
                });
            ws.fire_activity_completion(chat.pane, outcome.expect("reconciled completion"), cx);
        });
    }
    // A waiter's wake-up is the engine thread's signal: no polling sleep.
    let (watching, wake) = cx
        .update(|cx| RunsHost::open_run_wait(Some(RunsHost::run_scope(cx)), cx))
        .unwrap();
    loop {
        cx.run_until_parked();
        if cx.update(|cx| RunsHost::unreported_run_events(cx).len()) == 3 {
            break;
        }
        assert!(wake.recv().await.is_ok(), "the flow run ends");
    }
    cx.update(|cx| RunsHost::finish_run_wait(watching, false, cx));
    cx.run_until_parked();
    assert!(sent(cx).is_empty(), "the orchestrator is still busy");
    complete(orchestrator, cx);
    assert!(queued(cx).is_empty());
    let prompts = sent(cx);
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].lines().count(), 3);
    assert!(prompts[0].contains(&handle.to_string()));
    assert_eq!(prompts[0].matches("task:").count(), 2);
    cx.update(|cx| {
        let (wait, _) = RunsHost::open_run_wait(None, cx).unwrap();
        assert_eq!(
            RunsHost::poll_run_wait(wait, false, cx)
                .unwrap()
                .ended
                .len(),
            3
        );
        RunsHost::finish_run_wait(wait, true, cx);
    });
}

#[gpui::test]
fn pane_creation_recovers_events_created_before_effects_installation(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.update(|cx| produce("early-carry", cx));
    install(cx);
    let pane = crate::test_support::register_test_orchestrator(cx);
    let _probe = connect(pane, cx);
    cx.run_until_parked();
    let prompts = sent(cx);
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("flow:early-carry"));
}

#[path = "run_notices/regressions.rs"]
mod regressions;
