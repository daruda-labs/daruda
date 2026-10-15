use super::*;
use crate::runs::seed::{self, SeedEnd, SeedRun};
use crate::workspace::main_area::pane_input_ops::{PaneTextInput, PaneTextIntent};
use crate::workspace::tests::build_workspace;
use daruda_chat::view::AgentSessionStatus;
use daruda_store::tasks::Lifecycle;
use daruda_store::tasks::machine::UndeliveredReason;
use gpui::{AppContext as _, TestAppContext};

fn preparing(
    ws: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> (PaneId, ExecutionRef) {
    let pane = ws.create_agent_chat_pane(
        Some(PaneCwd::Local(std::env::temp_dir())),
        None,
        ws.mirrors.agents[0].id.clone(),
        None,
        window,
        cx,
    );
    let pane_id = pane.id;
    pane.agent_chat_view()
        .unwrap()
        .update(cx, |view, cx| view.begin_connect(None, cx));
    let run = SeedRun::chat(std::env::temp_dir());
    let id = seed::task(
        seed::spec(Default::default(), "Delivery"),
        Some((&run, SeedEnd::Dispatching)),
        cx,
    );
    ws.active_runtime_mut().panes.push(pane);
    let exec = run.exec(&id);
    ws.bind_task_chat_execution(&exec, pane_id, cx).unwrap();
    (pane_id, exec)
}

#[gpui::test]
fn queued_task_is_not_running_until_its_tagged_prompt_is_sent(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let (pane, exec) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let (pane, exec) = preparing(ws, window, cx);
                ws.send_agent_task_prompt_text(pane, "do the task".into(), Some(exec.clone()), cx);
                assert!(matches!(
                    cx.global::<GlobalTasks>()
                        .get(&exec.task_id)
                        .unwrap()
                        .lifecycle,
                    Lifecycle::Dispatching { .. }
                ));
                (pane, exec)
            })
        })
        .unwrap();
    cx.run_until_parked();
    let _probe = workspace.update(cx, |ws, cx| {
        let (handle, probe) = daruda_acp::AcpSessionHandle::detached_for_test();
        ws.agent_chat_view(pane).unwrap().update(cx, |view, _| {
            view.attach_handle(handle);
            view.set_status_for_test(AgentSessionStatus::Connected);
            view.set_session_id_for_test(Some("sent-session".into()));
        });
        ws.fold_agent_chat_event(
            pane,
            daruda_acp::AcpEvent::Connected {
                program: None,
                session_id: "sent-session".into(),
                modes: None,
                config_options: Vec::new(),
                capabilities: Default::default(),
                login_methods: Vec::new(),
            },
            cx,
        );
        probe
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let task = cx.global::<GlobalTasks>().get(&exec.task_id).unwrap();
        assert!(matches!(task.lifecycle, Lifecycle::Running { .. }));
        assert_eq!(
            task.execution.as_ref().unwrap().session_id.as_deref(),
            Some("sent-session")
        );
    });
}

#[gpui::test]
fn a_tagged_prompt_connection_failure_never_becomes_running(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let exec = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let (pane, exec) = preparing(ws, window, cx);
                ws.send_agent_task_prompt_text(pane, "do the task".into(), Some(exec.clone()), cx);
                ws.agent_chat_stream_ended(pane, cx);
                exec
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(matches!(
            cx.global::<GlobalTasks>()
                .get(&exec.task_id)
                .unwrap()
                .lifecycle,
            Lifecycle::Undelivered {
                reason: UndeliveredReason::ConnectFailed,
                ..
            }
        ))
    });
}

#[gpui::test]
fn a_task_prompt_does_not_interpret_clear_as_a_local_command(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let (pane, exec) = preparing(ws, window, cx);
            ws.send_agent_task_prompt_text(pane, "/clear".into(), Some(exec), cx);
            assert!(
                ws.agent_chat_view(pane)
                    .unwrap()
                    .read(cx)
                    .queue()
                    .has_pending()
            );
        })
    })
    .unwrap();
}

#[gpui::test]
fn task_delivery_rejects_a_non_submit_instead_of_losing_its_tag(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let (pane, exec) = preparing(ws, window, cx);
            assert!(!ws.deliver_text_to_pane(
                pane,
                PaneTextInput {
                    body: "do the task".into(),
                    intent: PaneTextIntent::Literal,
                    task: Some(exec)
                },
                window,
                cx
            ));
        })
    })
    .unwrap();
}

#[derive(Clone, Copy, Debug)]
enum LossPath {
    Remove,
    Clear,
    Reset,
    Close,
    QueueFull,
}

#[gpui::test]
fn every_queued_prompt_loss_commits_one_honest_terminal_fact(cx: &mut TestAppContext) {
    use daruda_chat::view::PromptDispatch;
    let (window, workspace) = build_workspace(cx);
    for (path, reason) in [
        (LossPath::Remove, UndeliveredReason::RemovedByUser),
        (LossPath::Clear, UndeliveredReason::RemovedByUser),
        (LossPath::Reset, UndeliveredReason::SessionTornDown),
        (LossPath::Close, UndeliveredReason::PaneClosed),
        (LossPath::QueueFull, UndeliveredReason::QueueFull),
    ] {
        let (exec, before) = cx
            .update_window(window.into(), |_, window, cx| {
                workspace.update(cx, |ws, cx| {
                    let (pane, exec) = preparing(ws, window, cx);
                    if matches!(path, LossPath::QueueFull) {
                        while ws.send_agent_task_prompt_text(pane, "unrelated".into(), None, cx)
                            == Some(PromptDispatch::Queued)
                        {}
                    }
                    ws.send_agent_task_prompt_text(
                        pane,
                        "do the task".into(),
                        Some(exec.clone()),
                        cx,
                    );
                    let before = RunsHost::sequence_for_test(cx);
                    let view = ws.agent_chat_view(pane).cloned().unwrap();
                    match path {
                        LossPath::Remove => {
                            let id = view.read(cx).queue().last_prompt_id().unwrap();
                            ws.remove_queued_prompt(pane, id, cx);
                        }
                        LossPath::Clear => ws.clear_queued_prompts(pane, window, cx),
                        LossPath::Reset => ws.reset_agent_chat_session(pane, cx),
                        LossPath::Close => ws.forget_panes(&[pane], cx),
                        LossPath::QueueFull => {}
                    }
                    (exec, before)
                })
            })
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let task = cx.global::<GlobalTasks>().get(&exec.task_id).unwrap();
            assert!(matches!(task.lifecycle, Lifecycle::Undelivered { reason: actual, .. } if actual == reason), "{path:?}: {:?}", task.lifecycle);
            assert_eq!(RunsHost::sequence_for_test(cx), before + 1, "{path:?}");
        });
    }
}

#[gpui::test]
fn immediate_sending_and_a_late_completion_are_ordered_after_preparation(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let exec = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let (pane, exec) = preparing(ws, window, cx);
                let (handle, _probe) = daruda_acp::AcpSessionHandle::detached_for_test();
                ws.agent_chat_view(pane).unwrap().update(cx, |view, _| {
                    view.attach_handle(handle);
                    view.set_status_for_test(AgentSessionStatus::Connected);
                });
                ws.send_agent_task_prompt_text(pane, "do the task".into(), Some(exec.clone()), cx);
                cx.defer(move |cx| {
                    RunsHost::settle_execution(
                        exec.clone(),
                        daruda_store::tasks::machine::SettleOutcome::Completed {
                            end_reason: daruda_store::tasks::SessionEndReason::Stop,
                        },
                        cx,
                    );
                });
                RunsHost::pane_execution(ws.run_pane(pane), cx)
                    .cloned()
                    .unwrap()
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(matches!(
            cx.global::<GlobalTasks>()
                .get(&exec.task_id)
                .unwrap()
                .lifecycle,
            Lifecycle::Done { .. }
        ))
    });
}

#[gpui::test]
fn running_chat_uses_its_frozen_surface_after_the_task_is_edited(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let id = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let (pane, exec) = preparing(ws, window, cx);
                seed::commit_now(
                    vec![TaskEvent::Dispatched {
                        exec: exec.clone(),
                        session_id: None,
                        at: chrono::Utc::now(),
                    }],
                    cx,
                );
                let spec = &cx.global::<GlobalTasks>().get(&exec.task_id).unwrap().spec;
                let edit = daruda_store::tasks::machine::TaskEdit {
                    title: spec.title.clone(),
                    prompt: spec.prompt.clone(),
                    notes: spec.notes.clone(),
                    auto_execute: spec.auto_execute,
                    agent_surface: daruda_store::tasks::TaskAgentSurface::Terminal,
                    base_worktree_path: None,
                    branch: None,
                    run_in: None,
                    project: None,
                };
                seed::commit_now(
                    vec![TaskEvent::Edited {
                        id: exec.task_id.clone(),
                        edit,
                        at: chrono::Utc::now(),
                    }],
                    cx,
                );
                assert_eq!(ws.task_chat_execution(pane, cx), Some(exec.clone()));
                ws.settle_agent_chat_task(
                    pane,
                    daruda_store::tasks::machine::SettleOutcome::Stopped {
                        cause: daruda_store::tasks::machine::StopCause::UserStop,
                    },
                    cx,
                );
                exec.task_id
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(matches!(
            cx.global::<GlobalTasks>().get(&id).unwrap().lifecycle,
            Lifecycle::Interrupted {
                cause: daruda_store::tasks::machine::InterruptCause::UserStop,
                ..
            }
        ))
    });
}

#[gpui::test]
fn stopping_a_queued_task_parks_it_without_claiming_it_was_sent(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let exec = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let (pane, exec) = preparing(ws, window, cx);
                ws.agent_chat_view(pane)
                    .unwrap()
                    .update(cx, |view, _| view.set_turn_in_flight());
                ws.send_agent_task_prompt_text(pane, "do the task".into(), Some(exec.clone()), cx);
                ws.cancel_agent_turn(pane, cx);
                exec
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(matches!(
            cx.global::<GlobalTasks>()
                .get(&exec.task_id)
                .unwrap()
                .lifecycle,
            Lifecycle::Dispatching { .. }
        ))
    });
}

#[gpui::test]
fn resetting_a_sent_task_records_a_failed_session_not_a_lost_prompt(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let exec = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let (pane, exec) = preparing(ws, window, cx);
                seed::commit_now(
                    vec![TaskEvent::Dispatched {
                        exec: exec.clone(),
                        session_id: None,
                        at: chrono::Utc::now(),
                    }],
                    cx,
                );
                ws.reset_agent_chat_session(pane, cx);
                exec
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(matches!(
            cx.global::<GlobalTasks>()
                .get(&exec.task_id)
                .unwrap()
                .lifecycle,
            Lifecycle::Error {
                cause: daruda_store::tasks::machine::ErrorCause::SessionFailed,
                ..
            }
        ))
    });
}

#[gpui::test]
fn a_read_only_pane_cannot_silently_accept_a_task(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let exec = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let (pane, exec) = preparing(ws, window, cx);
                ws.agent_chat_view(pane).unwrap().update(cx, |view, _| {
                    view.set_access(daruda_store::tasks::AgentChatAccess::CliSnapshot(
                        exec.clone(),
                    ))
                });
                assert!(!ws.deliver_text_to_pane(
                    pane,
                    PaneTextInput {
                        body: "do the task".into(),
                        intent: PaneTextIntent::Command { submit: true },
                        task: Some(exec.clone())
                    },
                    window,
                    cx
                ));
                exec
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(matches!(
            cx.global::<GlobalTasks>()
                .get(&exec.task_id)
                .unwrap()
                .lifecycle,
            Lifecycle::Error {
                cause: daruda_store::tasks::machine::ErrorCause::StartFailed(
                    daruda_store::tasks::machine::StartFailure::PaneUnavailable
                ),
                ..
            }
        ))
    });
}

#[gpui::test]
fn a_missing_agent_is_refused_before_any_prompt_can_be_sent(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let exec = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let (pane, exec) = preparing(ws, window, cx);
                ws.mirrors.agents = Default::default();
                ws.connect_agent_chat(pane, PaneCwd::Local(std::env::temp_dir()), None, cx);
                exec
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(matches!(
            cx.global::<GlobalTasks>()
                .get(&exec.task_id)
                .unwrap()
                .lifecycle,
            Lifecycle::Error {
                cause: daruda_store::tasks::machine::ErrorCause::StartFailed(
                    daruda_store::tasks::machine::StartFailure::AgentMissing
                ),
                ..
            }
        ))
    });
}

#[gpui::test]
fn a_connection_plan_that_cannot_be_prepared_is_not_left_dispatching(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let exec = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let (pane, exec) = preparing(ws, window, cx);
                ws.active_runtime_mut()
                    .panes
                    .retain(|present| present.id != pane);
                ws.connect_agent_chat(pane, PaneCwd::Local(std::env::temp_dir()), None, cx);
                exec
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(matches!(
            cx.global::<GlobalTasks>()
                .get(&exec.task_id)
                .unwrap()
                .lifecycle,
            Lifecycle::Error {
                cause: daruda_store::tasks::machine::ErrorCause::StartFailed(
                    daruda_store::tasks::machine::StartFailure::ConnectUnavailable
                ),
                ..
            }
        ))
    });
}

#[gpui::test]
fn sending_and_closing_in_one_update_keeps_the_sent_session_and_settles_once(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = build_workspace(cx);
    let (exec, before) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let (pane, exec) = preparing(ws, window, cx);
                let (handle, _probe) = daruda_acp::AcpSessionHandle::detached_for_test();
                ws.agent_chat_view(pane).unwrap().update(cx, |view, _| {
                    view.attach_handle(handle);
                    view.set_status_for_test(AgentSessionStatus::Connected);
                    view.set_session_id_for_test(Some("sent-before-close".into()));
                });
                let before = RunsHost::sequence_for_test(cx);
                ws.send_agent_task_prompt_text(pane, "task prompt".into(), Some(exec.clone()), cx);
                ws.forget_panes(&[pane], cx);
                ws.active_runtime_mut()
                    .panes
                    .retain(|present| present.id != pane);
                (exec, before)
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let task = cx.global::<GlobalTasks>().get(&exec.task_id).unwrap();
        assert!(
            matches!(
                task.lifecycle,
                Lifecycle::Error {
                    cause: daruda_store::tasks::machine::ErrorCause::SessionFailed,
                    ..
                }
            ),
            "{:?}, sequence {}",
            task.lifecycle,
            RunsHost::sequence_for_test(cx)
        );
        assert_eq!(
            task.execution.as_ref().unwrap().session_id.as_deref(),
            Some("sent-before-close")
        );
        assert_eq!(RunsHost::sequence_for_test(cx), before + 2);
    });
}

#[gpui::test]
fn removing_a_prompt_then_closing_keeps_the_first_loss_reason(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let exec = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let (pane, exec) = preparing(ws, window, cx);
                ws.send_agent_task_prompt_text(pane, "task prompt".into(), Some(exec.clone()), cx);
                let id = ws
                    .agent_chat_view(pane)
                    .unwrap()
                    .read(cx)
                    .queue()
                    .last_prompt_id()
                    .unwrap();
                ws.remove_queued_prompt(pane, id, cx);
                ws.forget_panes(&[pane], cx);
                ws.active_runtime_mut()
                    .panes
                    .retain(|present| present.id != pane);
                exec
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(matches!(
            cx.global::<GlobalTasks>()
                .get(&exec.task_id)
                .unwrap()
                .lifecycle,
            Lifecycle::Undelivered {
                reason: UndeliveredReason::RemovedByUser,
                ..
            }
        ))
    });
}

#[gpui::test]
fn a_prompt_start_failure_drops_the_queue_without_a_running_transition(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let (pane, exec) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let (pane, exec) = preparing(ws, window, cx);
                ws.send_agent_task_prompt_text(pane, "task prompt".into(), Some(exec.clone()), cx);
                ws.connect_agent_chat(pane, PaneCwd::Local(std::env::temp_dir()), None, cx);
                (pane, exec)
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(matches!(
            cx.global::<GlobalTasks>()
                .get(&exec.task_id)
                .unwrap()
                .lifecycle,
            Lifecycle::Undelivered {
                reason: UndeliveredReason::StartFailed,
                ..
            }
        ))
    });
    workspace.read_with(cx, |ws, cx| {
        assert!(
            !ws.agent_chat_view(pane)
                .unwrap()
                .read(cx)
                .queue()
                .has_pending()
        )
    });
}
