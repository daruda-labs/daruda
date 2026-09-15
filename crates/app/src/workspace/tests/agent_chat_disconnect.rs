//! Stream EOF must settle only the activity owned by the disconnected pane.

use gpui::{AppContext as _, BorrowAppContext as _, Context, TestAppContext, Window};

use super::build_workspace;
use crate::agent::tasks_global::GlobalTasks;
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::view::AgentSessionStatus;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_store::project::PaneCwd;
use daruda_store::tasks::{Task, TaskState};

fn pane(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> PaneId {
    let pane = ws.create_agent_chat_pane(
        Some(PaneCwd::Local(std::env::temp_dir())),
        None,
        ws.agents[0].id.clone(),
        None,
        window,
        cx,
    );
    let id = pane.id;
    pane.agent_chat_view().unwrap().update(cx, |view, _| {
        view.status = AgentSessionStatus::Connected;
    });
    ws.active_runtime_mut().panes.push(pane);
    id
}

fn running_task(cx: &mut Context<Workspace>) -> String {
    let mut task = Task::new("ACP task".into(), "prompt".into(), None);
    task.state = TaskState::Running {
        worktree_path: std::env::temp_dir(),
    };
    let id = task.id.clone();
    cx.update_global::<GlobalTasks, _>(|tasks, _| {
        tasks.add(task);
    });
    id
}

#[gpui::test]
fn idle_agent_chat_eof_leaves_another_same_cwd_task_running(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let idle = pane(ws, window, cx);
            let active = pane(ws, window, cx);
            let task = running_task(cx);
            ws.agent_chat_view(active).unwrap().update(cx, |view, _| {
                view.set_turn_in_flight();
                view.reconcile_activity(std::time::Instant::now());
            });

            ws.agent_chat_stream_ended(idle, cx);

            assert!(matches!(
                ws.agent_chat_view(idle).unwrap().read(cx).status,
                AgentSessionStatus::Error {
                    remedy: daruda_acp::Remedy::Retry,
                    ..
                }
            ));
            assert!(ws.agent_chat_view(active).unwrap().read(cx).is_busy());
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&task).unwrap().state,
                TaskState::Running { .. }
            ));
        });
    })
    .unwrap();
}

#[gpui::test]
fn active_agent_chat_eof_completes_once_through_the_activity_edge(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = pane(ws, window, cx);
            let view = ws.agent_chat_view(pane).cloned().unwrap();
            let task = running_task(cx);
            view.update(cx, |view, _| {
                view.set_turn_in_flight();
                view.reconcile_activity(std::time::Instant::now());
            });

            ws.agent_chat_stream_ended(pane, cx);

            assert!(matches!(
                cx.global::<GlobalTasks>().get(&task).unwrap().state,
                TaskState::Error { .. }
            ));
            assert!(!view.read(cx).is_busy());
            assert!(view.read(cx).activity.pending_completion.is_none());
            let next_task = running_task(cx);
            ws.agent_chat_stream_ended(pane, cx);
            ws.pulse_agent_chats(cx);
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&next_task).unwrap().state,
                TaskState::Running { .. }
            ));
        });
    })
    .unwrap();
}

#[gpui::test]
fn agent_chat_startup_eof_without_a_prompt_leaves_other_tasks_running(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = pane(ws, window, cx);
            let task = running_task(cx);
            ws.agent_chat_view(pane).unwrap().update(cx, |view, cx| {
                view.begin_connect(Some("saved-session".into()), cx);
            });
            ws.agent_chat_stream_ended(pane, cx);
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&task).unwrap().state,
                TaskState::Running { .. }
            ));
        });
    })
    .unwrap();
}

#[gpui::test]
fn agent_chat_startup_eof_still_fails_a_task_waiting_to_dispatch(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = pane(ws, window, cx);
            let task = running_task(cx);
            ws.agent_chat_view(pane).unwrap().update(cx, |view, cx| {
                view.begin_connect(Some("saved-session".into()), cx);
                view.send_prompt_text("task prompt".into(), cx);
            });
            ws.agent_chat_stream_ended(pane, cx);
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&task).unwrap().state,
                TaskState::Error { .. }
            ));
        });
    })
    .unwrap();
}
