//! Task conversations retain exact identity and never share completion ownership.

use super::*;
use crate::workspace::main_area::agent_chat_pane::view::AgentSessionStatus;
use crate::workspace::tests::build_workspace;
use daruda_store::tasks::{SessionEndReason, Task, TaskState};
use gpui::{AppContext as _, TestAppContext};

fn task(cx: &mut Context<Workspace>) -> String {
    let mut task = Task::new(
        daruda_store::project::ProjectUuid::default(),
        "Task".into(),
        "Prompt".into(),
        None,
    );
    task.state = TaskState::Running {
        worktree_path: std::env::temp_dir(),
    };
    let id = task.id.clone();
    cx.update_global::<GlobalTasks, _>(|tasks, _| {
        tasks.add(task);
    });
    id
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
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = pane(ws, window, cx);
            let owned = task(cx);
            let unrelated = task(cx);
            ws.bind_task_chat_execution(&owned, pane, cx);
            ws.apply_agent_chat_task_ended(pane, SessionEndReason::Stop, cx);
            let tasks = cx.global::<GlobalTasks>();
            assert!(matches!(
                tasks.get(&owned).unwrap().state,
                TaskState::Done { .. }
            ));
            assert!(matches!(
                tasks.get(&unrelated).unwrap().state,
                TaskState::Running { .. }
            ));
            assert!(tasks.get(&owned).unwrap().finished_at.is_some());
            ws.apply_agent_chat_task_ended(pane, SessionEndReason::Error, cx);
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&owned).unwrap().state,
                TaskState::Done { .. }
            ));
        })
    })
    .unwrap();
}

#[gpui::test]
fn task_chat_open_selects_existing_tab_without_duplicate_or_reconnect(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = pane(ws, window, cx);
            let id = task(cx);
            ws.bind_task_chat_execution(&id, pane, cx);
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
            ws.bind_task_chat_execution(&first, first_pane, cx);
            ws.bind_task_chat_execution(&second, second_pane, cx);
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
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = pane(ws, window, cx);
            let id = task(cx);
            ws.bind_task_chat_execution(&id, pane, cx);
            assert!(ws.is_task_chat_restore(pane, cx));
            ws.agent_chat_view(pane).unwrap().update(cx, |view, _| {
                view.set_session_id_for_test(Some("another-session".into()))
            });
            ws.record_task_chat_session(pane, cx);
            ws.apply_agent_chat_task_ended(pane, SessionEndReason::Error, cx);
            let task = cx.global::<GlobalTasks>().get(&id).unwrap();
            assert!(matches!(task.state, TaskState::Running { .. }));
            assert_eq!(
                task.execution.as_ref().unwrap().session_id.as_deref(),
                Some("task-session")
            );
            assert!(!ws.is_task_chat_restore(pane, cx));
        })
    })
    .unwrap();
}

#[gpui::test]
fn task_chat_history_has_no_completion_ownership(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = pane(ws, window, cx);
            let id = task(cx);
            ws.bind_task_chat_execution(&id, pane, cx);
            ws.active_runtime_mut()
                .panes
                .iter_mut()
                .find(|p| p.id == pane)
                .unwrap()
                .agent_chat_content_mut()
                .unwrap()
                .task_run = None;
            assert!(ws.is_task_chat_restore(pane, cx));
            ws.apply_agent_chat_task_ended(pane, SessionEndReason::Error, cx);
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&id).unwrap().state,
                TaskState::Running { .. }
            ));
        })
    })
    .unwrap();
}

#[gpui::test]
fn cancelling_task_stops_its_turn_and_discards_queued_prompts(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = pane(ws, window, cx);
            let id = task(cx);
            ws.bind_task_chat_execution(&id, pane, cx);
            let view = ws.agent_chat_view(pane).cloned().unwrap();
            view.update(cx, |view, cx| {
                view.set_turn_in_flight();
                view.send_prompt_text("queued follow-up".into(), cx);
                view.tick_activity(std::time::Instant::now(), cx);
            });

            ws.cancel_task(&id, cx);

            assert!(matches!(
                cx.global::<GlobalTasks>().get(&id).unwrap().state,
                TaskState::Cancelled { .. }
            ));
            assert!(!view.read(cx).is_busy());
            assert!(view.read(cx).queue().pending_prompts.is_empty());
            assert!(view.read(cx).queue().paused_prompts.is_empty());
        })
    })
    .unwrap();
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
            ws.bind_task_chat_execution(&id, pane, cx);
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
            assert!(chat.task_run.is_none());
            assert!(chat.view.read(cx).any_handle().is_none());
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&id).unwrap().state,
                TaskState::Running { .. }
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
            ws.bind_task_chat_execution(&id, pane, cx);
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
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = pane(ws, window, cx);
            let id = task(cx);
            ws.agent_chat_view(pane)
                .unwrap()
                .update(cx, |view, _| view.set_session_id_for_test(None));
            ws.bind_task_chat_execution(&id, pane, cx);
            ws.agent_chat_view(pane).unwrap().update(cx, |view, _| {
                view.set_session_id_for_test(Some("first-session".into()))
            });
            ws.record_task_chat_session(pane, cx);
            assert_eq!(
                cx.global::<GlobalTasks>()
                    .get(&id)
                    .unwrap()
                    .execution
                    .as_ref()
                    .unwrap()
                    .session_id
                    .as_deref(),
                Some("first-session")
            );
            cx.update_global::<GlobalTasks, _>(|tasks, _| {
                tasks.get_mut(&id).unwrap().execution.as_mut().unwrap().id = "retry-run".into()
            });
            ws.apply_agent_chat_task_ended(pane, SessionEndReason::Stop, cx);
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&id).unwrap().state,
                TaskState::Running { .. }
            ));
        })
    })
    .unwrap();
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
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            std::sync::Arc::make_mut(&mut ws.mirrors.agents)[0].launch =
                daruda_config::AgentLaunch::Raw("daruda-missing-test-adapter".into());
            let pane = pane(ws, window, cx);
            let id = task(cx);
            ws.bind_task_chat_execution(&id, pane, cx);
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
            assert!(chat.task_run.is_none());
            assert!(ws.is_task_chat_restore(reopened, cx));
            let count = ws.active_runtime().panes.len();
            ws.open_task_chat(&id, window, cx);
            assert_eq!(ws.active_runtime().panes.len(), count);
            assert_eq!(ws.active_runtime().focused_pane_id, reopened);
            ws.apply_agent_chat_task_ended(reopened, SessionEndReason::Error, cx);
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&id).unwrap().state,
                TaskState::Running { .. }
            ));
        })
    })
    .unwrap();
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
            ws.bind_task_chat_execution(&id, pane, cx);
            assert_eq!(ws.begin_task_chat_open(&id, window, cx), Ok(pane));
        })
    })
    .unwrap();
}
