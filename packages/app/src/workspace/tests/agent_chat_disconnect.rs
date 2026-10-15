//! Stream EOF must settle only the activity owned by the disconnected pane.

use gpui::{AppContext as _, Context, TestAppContext, Window};

use crate::runs::GlobalTasks;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_chat::view::AgentSessionStatus;
use daruda_store::project::PaneCwd;
use daruda_store::tasks::Lifecycle;

/// Commits land on the spot: these tests are about which pane settles a
/// task, not when the settle lands.
fn build_workspace(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
) {
    let built = super::build_workspace(cx);
    cx.update(crate::runs::RunsHost::inline_commits_for_test);
    built
}

fn pane(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> PaneId {
    let pane = ws.create_agent_chat_pane(
        Some(PaneCwd::Local(std::env::temp_dir())),
        None,
        ws.mirrors.agents[0].id.clone(),
        None,
        window,
        cx,
    );
    let id = pane.id;
    pane.agent_chat_view().unwrap().update(cx, |view, _| {
        view.set_status_for_test(AgentSessionStatus::Connected);
    });
    ws.active_runtime_mut().panes.push(pane);
    id
}

/// A task running in the shared temp cwd; `run_task_in_chat_for_test`
/// starts one in a pane instead.
fn running_task(cx: &mut Context<Workspace>) -> String {
    let spec = crate::runs::seed::spec(daruda_store::project::ProjectUuid::default(), "ACP task");
    let run = crate::runs::seed::SeedRun::chat(std::env::temp_dir());
    crate::runs::seed::task(spec, Some((&run, crate::runs::seed::SeedEnd::Running)), cx)
}

fn backlog_task(cx: &mut Context<Workspace>) -> String {
    let spec = crate::runs::seed::spec(daruda_store::project::ProjectUuid::default(), "ACP task");
    crate::runs::seed::task(spec, None, cx)
}

#[gpui::test]
fn idle_agent_chat_eof_leaves_another_same_cwd_task_running(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let idle = pane(ws, window, cx);
            let active = pane(ws, window, cx);
            let task = backlog_task(cx);
            ws.run_task_in_chat_for_test(&task, active, cx);
            ws.agent_chat_view(active).unwrap().update(cx, |view, cx| {
                view.set_turn_in_flight();
                view.tick_activity(std::time::Instant::now(), cx);
            });

            ws.agent_chat_stream_ended(idle, cx);

            assert!(matches!(
                ws.agent_chat_view(idle).unwrap().read(cx).status(),
                AgentSessionStatus::Error {
                    remedy: daruda_acp::Remedy::Retry,
                    ..
                }
            ));
            assert!(ws.agent_chat_view(active).unwrap().read(cx).is_busy());
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&task).unwrap().lifecycle,
                Lifecycle::Running { .. }
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
            let task = backlog_task(cx);
            ws.run_task_in_chat_for_test(&task, pane, cx);
            view.update(cx, |view, cx| {
                view.set_turn_in_flight();
                view.tick_activity(std::time::Instant::now(), cx);
            });

            ws.agent_chat_stream_ended(pane, cx);

            assert!(matches!(
                cx.global::<GlobalTasks>().get(&task).unwrap().lifecycle,
                Lifecycle::Error { .. }
            ));
            assert!(!view.read(cx).is_busy());
            assert!(!view.read(cx).owes_completion_for_test());
            let next_task = running_task(cx);
            ws.agent_chat_stream_ended(pane, cx);
            ws.pulse_agent_chats(cx);
            assert!(matches!(
                cx.global::<GlobalTasks>()
                    .get(&next_task)
                    .unwrap()
                    .lifecycle,
                Lifecycle::Running { .. }
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
                cx.global::<GlobalTasks>().get(&task).unwrap().lifecycle,
                Lifecycle::Running { .. }
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
            let task = backlog_task(cx);
            ws.run_task_in_chat_for_test(&task, pane, cx);
            ws.agent_chat_view(pane).unwrap().update(cx, |view, cx| {
                view.begin_connect(Some("saved-session".into()), cx);
                view.send_prompt_text("task prompt".into(), cx);
            });
            ws.agent_chat_stream_ended(pane, cx);
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&task).unwrap().lifecycle,
                Lifecycle::Error { .. }
            ));
        });
    })
    .unwrap();
}
