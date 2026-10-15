//! Task conversations retain exact identity and never share completion ownership.

use super::*;
use crate::runs::seed::commit_now;
use crate::workspace::tests::build_workspace;
use daruda_chat::view::AgentSessionStatus;
use daruda_store::tasks::machine::SettleOutcome;
use daruda_store::tasks::{Lifecycle, SessionEndReason, TaskExecutionSource};
use gpui::{AppContext as _, TestAppContext};

/// A Backlog task; `bind` starts its run in a pane.
fn task(cx: &mut Context<Workspace>) -> String {
    let spec = crate::runs::seed::spec(daruda_store::project::ProjectUuid::default(), "Task");
    crate::runs::seed::task(spec, None, cx)
}

fn bind(ws: &mut Workspace, id: &str, pane: PaneId, cx: &mut Context<Workspace>) -> ExecutionRef {
    ws.run_task_in_chat_for_test(id, pane, cx)
}

fn ended(reason: SessionEndReason) -> SettleOutcome {
    match reason {
        SessionEndReason::Error => SettleOutcome::Errored,
        end_reason => SettleOutcome::Completed { end_reason },
    }
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

fn pane(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> PaneId {
    let pane = ws.create_agent_chat_pane(
        Some(PaneCwd::Local(
            ws.active_lane()
                .map(|lane| lane.path.clone())
                .unwrap_or_else(std::env::temp_dir),
        )),
        Some("task-session".into()),
        ws.mirrors.agents[0].id.clone(),
        None,
        window,
        cx,
    );
    let id = pane.id;
    pane.agent_chat_view().unwrap().update(cx, |view, _| {
        view.set_status_for_test(AgentSessionStatus::Connected)
    });
    let tab_id = ws.alloc_id();
    ws.active_runtime_mut().panes.push(pane);
    ws.active_runtime_mut().tabs.push(TabEntry {
        id: tab_id,
        layout: PaneLayout::Pane(id),
        last_focused_pane: id,
        user_label: None,
    });
    id
}

#[gpui::test]
fn task_chat_completion_uses_execution_not_cwd(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let (pane, owned, unrelated) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = pane(ws, window, cx);
                let owned = task(cx);
                // Same cwd, its own run: the pane does not own it.
                let unrelated = crate::runs::seed::task(
                    crate::runs::seed::spec(Default::default(), "Unrelated"),
                    Some((
                        &crate::runs::seed::SeedRun::chat(std::env::temp_dir()),
                        crate::runs::seed::SeedEnd::Running,
                    )),
                    cx,
                );
                bind(ws, &owned, pane, cx);
                ws.settle_agent_chat_task(pane, ended(SessionEndReason::Stop), cx);
                (pane, owned, unrelated)
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert!(matches!(lifecycle(&owned, cx), Lifecycle::Done { .. }));
    assert!(matches!(
        lifecycle(&unrelated, cx),
        Lifecycle::Running { .. }
    ));
    cx.update(|cx| {
        assert!(
            cx.global::<GlobalTasks>()
                .get(&owned)
                .unwrap()
                .finished_at
                .is_some()
        )
    });
    workspace.update(cx, |ws, cx| {
        ws.settle_agent_chat_task(pane, ended(SessionEndReason::Error), cx)
    });
    cx.run_until_parked();
    assert!(matches!(lifecycle(&owned, cx), Lifecycle::Done { .. }));
}

#[gpui::test]
fn task_chat_open_selects_existing_tab_without_duplicate_or_reconnect(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = pane(ws, window, cx);
            let id = task(cx);
            bind(ws, &id, pane, cx);
            let count = ws.active_runtime().panes.len();
            ws.open_task_chat(&id, window, cx);
            ws.open_task_chat(&id, window, cx);
            let rt = ws.active_runtime();
            assert_eq!(rt.panes.len(), count);
            assert_eq!(rt.focused_pane_id, pane);
            assert!(rt.tabs[rt.active_tab_index].layout.contains(pane));
            assert!(matches!(
                ws.agent_chat_view(pane).unwrap().read(cx).status(),
                AgentSessionStatus::Connected
            ));
        })
    })
    .unwrap();
}

/// Tasks sharing one lane share its cwd; each must still reopen its own chat.
#[gpui::test]
fn task_chats_in_one_lane_each_reopen_their_own_pane(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let first_pane = pane(ws, window, cx);
            let second_pane = pane(ws, window, cx);
            ws.agent_chat_view(second_pane)
                .unwrap()
                .update(cx, |view, _| {
                    view.set_session_id_for_test(Some("second-session".into()))
                });
            let first = task(cx);
            let second = task(cx);
            bind(ws, &first, first_pane, cx);
            bind(ws, &second, second_pane, cx);
            ws.open_task_chat(&first, window, cx);
            assert_eq!(ws.active_runtime().focused_pane_id, first_pane);
            ws.open_task_chat(&second, window, cx);
            assert_eq!(ws.active_runtime().focused_pane_id, second_pane);
        })
    })
    .unwrap();
}

#[gpui::test]
fn task_chat_changed_session_cannot_replace_identity_or_finish_task(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let id = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = pane(ws, window, cx);
                let id = task(cx);
                bind(ws, &id, pane, cx);
                assert!(ws.is_task_chat_restore(pane, cx));
                ws.agent_chat_view(pane).unwrap().update(cx, |view, _| {
                    view.set_session_id_for_test(Some("another-session".into()))
                });
                ws.record_task_chat_session(pane, cx);
                ws.settle_agent_chat_task(pane, ended(SessionEndReason::Error), cx);
                assert!(!ws.is_task_chat_restore(pane, cx));
                id
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert!(matches!(lifecycle(&id, cx), Lifecycle::Running { .. }));
    cx.update(|cx| {
        let task = cx.global::<GlobalTasks>().get(&id).unwrap();
        assert_eq!(
            task.execution.as_ref().unwrap().session_id.as_deref(),
            Some("task-session")
        );
    });
}

#[gpui::test]
fn task_chat_history_has_no_completion_ownership(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let id = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = pane(ws, window, cx);
                let id = task(cx);
                bind(ws, &id, pane, cx);
                RunsHost::unbind_execution(ws.run_pane(pane), cx);
                assert!(ws.is_task_chat_restore(pane, cx));
                ws.settle_agent_chat_task(pane, ended(SessionEndReason::Error), cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert!(matches!(lifecycle(&id, cx), Lifecycle::Running { .. }));
}

#[gpui::test]
fn cancelling_task_stops_its_turn_and_discards_queued_prompts(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(window.into(), workspace.downgrade(), cx)
    });
    let (id, view) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = pane(ws, window, cx);
                let id = task(cx);
                bind(ws, &id, pane, cx);
                let view = ws.agent_chat_view(pane).cloned().unwrap();
                view.update(cx, |view, cx| {
                    view.set_turn_in_flight();
                    view.send_prompt_text("queued follow-up".into(), cx);
                    view.tick_activity(std::time::Instant::now(), cx);
                });

                ws.cancel_task(&id, cx);
                (id, view)
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert!(matches!(lifecycle(&id, cx), Lifecycle::Cancelled { .. }));
    cx.update(|cx| {
        assert!(!view.read(cx).is_busy());
        assert!(view.read(cx).queue().in_strip_order().next().is_none());
    });
}

#[gpui::test]
fn another_window_cancels_a_task_after_its_cancel_commits(cx: &mut TestAppContext) {
    let (first_window, first) = build_workspace(cx);
    let (second_window, second) = build_workspace(cx);
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(
            first_window.into(),
            first.downgrade(),
            cx,
        );
        crate::window_registry::WindowRegistry::register(
            second_window.into(),
            second.downgrade(),
            cx,
        );
    });
    let (id, view) = cx
        .update_window(first_window.into(), |_, window, cx| {
            first.update(cx, |ws, cx| {
                let pane = pane(ws, window, cx);
                let id = task(cx);
                bind(ws, &id, pane, cx);
                let view = ws.agent_chat_view(pane).cloned().unwrap();
                view.update(cx, |view, cx| {
                    view.set_turn_in_flight();
                    view.tick_activity(std::time::Instant::now(), cx);
                });
                (id, view)
            })
        })
        .unwrap();
    second.update(cx, |ws, cx| {
        ws.cancel_task(&id, cx);
        assert!(view.read(cx).is_busy());
        assert!(matches!(
            cx.global::<GlobalTasks>().get(&id).unwrap().lifecycle,
            Lifecycle::Running { .. }
        ));
    });
    cx.run_until_parked();
    assert!(matches!(lifecycle(&id, cx), Lifecycle::Cancelled { .. }));
    cx.update(|cx| assert!(!view.read(cx).is_busy()));
}

#[gpui::test]
fn task_chat_disk_restore_preserves_missing_agent_and_account(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(project),
    );
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = pane(ws, window, cx);
            let id = task(cx);
            let account = daruda_store::accounts::AccountId(uuid::Uuid::new_v4());
            let chat = ws
                .active_runtime_mut()
                .panes
                .iter_mut()
                .find(|p| p.id == pane)
                .unwrap()
                .agent_chat_content_mut()
                .unwrap();
            chat.agent_id = "removed-agent".into();
            chat.account = AccountSelection::from_persisted(Some(account));
            chat.view.update(cx, |view, _| {
                view.set_agent_id_for_test("removed-agent".into())
            });
            bind(ws, &id, pane, cx);
            let (state, projects) = ws.snapshot_for_disk(cx);
            ws.restore_from_disk(&state, &projects, window, cx);
            let chat = ws
                .main_area
                .runtimes
                .values()
                .flat_map(|rt| rt.panes.iter())
                .filter_map(Pane::agent_chat_content)
                .find(|chat| chat.view.read(cx).session_id() == Some("task-session"))
                .unwrap();
            assert_eq!(chat.agent_id, "removed-agent");
            assert_eq!(chat.account.to_persisted(), Some(account));
            assert!(
                !ws.main_area
                    .runtimes
                    .values()
                    .flat_map(|rt| &rt.panes)
                    .filter(|pane| pane
                        .agent_chat_content()
                        .is_some_and(|content| content.view == chat.view))
                    .any(|pane| RunsHost::pane_execution(ws.run_pane(pane.id), cx).is_some())
            );
            assert!(chat.view.read(cx).any_handle().is_none());
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&id).unwrap().lifecycle,
                Lifecycle::Running { .. }
            ));
        })
    })
    .unwrap();
}

#[gpui::test]
fn task_chat_open_finds_parked_project(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(project),
    );
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = pane(ws, window, cx);
            let owner_lane = ws.active;
            let id = task(cx);
            bind(ws, &id, pane, cx);
            let other = tempfile::tempdir().unwrap();
            ws.add_project(other.path().to_path_buf(), window, cx);
            assert_ne!(ws.active, owner_lane);
            ws.open_task_chat(&id, window, cx);
            assert_eq!(ws.active, owner_lane);
            assert_eq!(ws.active_runtime().focused_pane_id, pane);
        })
    })
    .unwrap();
}

#[gpui::test]
fn task_chat_pending_identity_records_once_and_old_run_cannot_finish_retry(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = build_workspace(cx);
    let (pane, id, first) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = pane(ws, window, cx);
                let id = task(cx);
                ws.agent_chat_view(pane)
                    .unwrap()
                    .update(cx, |view, _| view.set_session_id_for_test(None));
                let first = bind(ws, &id, pane, cx);
                ws.agent_chat_view(pane).unwrap().update(cx, |view, _| {
                    view.set_session_id_for_test(Some("first-session".into()))
                });
                ws.record_task_chat_session(pane, cx);
                (pane, id, first)
            })
        })
        .unwrap();
    cx.run_until_parked();
    let session = cx.update(|cx| {
        cx.global::<GlobalTasks>()
            .get(&id)
            .unwrap()
            .execution
            .as_ref()
            .unwrap()
            .session_id
            .clone()
    });
    assert_eq!(session.as_deref(), Some("first-session"));
    // A retry: the first run ends and a new one is requested.
    cx.update(|cx| {
        let at = chrono::Utc::now();
        commit_now(
            vec![
                TaskEvent::CancelRequested { exec: first, at },
                TaskEvent::Reopened {
                    id: id.clone(),
                    run_in: Default::default(),
                    at,
                },
                TaskEvent::DispatchRequested {
                    id: id.clone(),
                    execution_id: "retry-run".into(),
                    source: TaskExecutionSource::AgentChat,
                    at,
                },
            ],
            cx,
        )
    });
    workspace.update(cx, |ws, cx| {
        ws.settle_agent_chat_task(pane, ended(SessionEndReason::Stop), cx)
    });
    cx.run_until_parked();
    assert!(
        matches!(lifecycle(&id, cx), Lifecycle::Preparing { exec, .. } if exec.execution_id == "retry-run"),
        "the old run's pane does not settle the retry"
    );
}

#[gpui::test]
fn task_chat_closed_pane_reopens_exact_session_without_execution_ownership(
    cx: &mut TestAppContext,
) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(project),
    );
    let id = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                std::sync::Arc::make_mut(&mut ws.mirrors.agents)[0].launch =
                    daruda_config::AgentLaunch::Raw("daruda-missing-test-adapter".into());
                let pane = pane(ws, window, cx);
                let id = task(cx);
                bind(ws, &id, pane, cx);
                ws.active_runtime_mut().panes.retain(|p| p.id != pane);
                ws.active_runtime_mut()
                    .tabs
                    .retain(|tab| !tab.layout.contains(pane));
                ws.open_task_chat(&id, window, cx);
                let reopened = ws.active_runtime().focused_pane_id;
                assert_ne!(reopened, pane);
                let chat = ws
                    .active_runtime()
                    .panes
                    .iter()
                    .find(|p| p.id == reopened)
                    .unwrap()
                    .agent_chat_content()
                    .unwrap();
                assert_eq!(chat.view.read(cx).session_id(), Some("task-session"));
                assert!(
                    !ws.main_area
                        .runtimes
                        .values()
                        .flat_map(|rt| &rt.panes)
                        .filter(|pane| pane
                            .agent_chat_content()
                            .is_some_and(|content| content.view == chat.view))
                        .any(|pane| RunsHost::pane_execution(ws.run_pane(pane.id), cx).is_some())
                );
                assert!(ws.is_task_chat_restore(reopened, cx));
                let count = ws.active_runtime().panes.len();
                ws.open_task_chat(&id, window, cx);
                assert_eq!(ws.active_runtime().panes.len(), count);
                assert_eq!(ws.active_runtime().focused_pane_id, reopened);
                ws.settle_agent_chat_task(reopened, ended(SessionEndReason::Error), cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert!(matches!(
        lifecycle(&id, cx),
        Lifecycle::Error {
            cause: daruda_store::tasks::machine::ErrorCause::SessionFailed,
            ..
        }
    ));
}

/// Opening answers with the pane it brought up, or why it could not.
#[gpui::test]
fn task_chat_open_answers_with_the_pane_or_why_not(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            assert_eq!(
                ws.begin_task_chat_open("missing", window, cx),
                Err(TaskChatError::NotFound)
            );
            let never_ran = task(cx);
            assert_eq!(
                ws.begin_task_chat_open(&never_ran, window, cx),
                Err(TaskChatError::NoSession)
            );
            let pane = pane(ws, window, cx);
            let id = task(cx);
            bind(ws, &id, pane, cx);
            assert_eq!(ws.begin_task_chat_open(&id, window, cx), Ok(pane));
        })
    })
    .unwrap();
}
