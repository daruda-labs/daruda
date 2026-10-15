//! A start answers with its outcome, and a refusal answers before git runs.

use std::path::PathBuf;

use super::{TaskStartError, TaskStarted};
use crate::runs::seed::{SeedEnd, SeedRun};
use crate::workspace::Workspace;
use crate::workspace::tests::build_workspace_with;
use daruda_store::project::ProjectUuid;
use daruda_store::tasks::{TaskAgentSurface, TaskRunIn, TaskSpec};
use gpui::{AppContext as _, Context, TestAppContext};

fn add(task: TaskSpec, cx: &mut Context<Workspace>) -> String {
    crate::runs::seed::task(task, None, cx)
}

fn task(project: ProjectUuid) -> TaskSpec {
    let mut task = crate::runs::seed::spec(project, "T");
    task.agent_surface = TaskAgentSurface::Terminal;
    task
}

async fn dispatch_while_switching_lanes(
    surface: TaskAgentSurface,
    release: bool,
    cx: &mut TestAppContext,
) {
    use crate::runs::{GlobalTasks, RunsHost};
    use daruda_store::tasks::{ExecutionRef, Lifecycle, TaskEvent, TaskExecutionSource};
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    let (id, pane, active, focused, dispatched, mut probe) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let mut spec = task(ws.projects[0].uuid);
                spec.agent_surface = surface;
                let id = add(spec, cx);
                let exec = ExecutionRef {
                    task_id: id.clone(),
                    execution_id: "delivery".into(),
                };
                crate::runs::seed::commit_now(
                    vec![TaskEvent::DispatchRequested {
                        id: id.clone(),
                        execution_id: exec.execution_id.clone(),
                        source: if surface == TaskAgentSurface::Terminal {
                            TaskExecutionSource::ClaudeCli {
                                transcript_path: None,
                                process: Default::default(),
                            }
                        } else {
                            TaskExecutionSource::AgentChat
                        },
                        at: chrono::Utc::now(),
                    }],
                    cx,
                );
                let (pane, probe) = if surface == TaskAgentSurface::Terminal {
                    (ws.active_runtime().panes[0].id, None)
                } else {
                    let pane = ws.create_agent_chat_pane(
                        Some(daruda_store::project::PaneCwd::Local(root.path().into())),
                        None,
                        ws.mirrors.agents[0].id.clone(),
                        None,
                        window,
                        cx,
                    );
                    let pane_id = pane.id;
                    let (handle, probe) = daruda_acp::AcpSessionHandle::detached_for_test();
                    pane.agent_chat_view().unwrap().update(cx, |view, _| {
                        view.attach_handle(handle);
                        view.set_status_for_test(daruda_chat::view::AgentSessionStatus::Connected);
                        view.set_session_id_for_test(Some("parked-session".into()));
                    });
                    ws.active_runtime_mut().panes.push(pane);
                    (pane_id, Some(probe))
                };
                let dispatched = ws
                    .dispatch_claude_for_task(&exec, root.path(), "main", pane, window, cx)
                    .unwrap();
                if release {
                    RunsHost::release_execution(
                        ws.run_pane(pane),
                        daruda_store::tasks::machine::UndeliveredReason::SessionTornDown,
                        cx,
                    );
                }
            if !release {
                ws.add_project(other.path().into(), window, cx);
                assert!(!ws.deliver_text_to_pane(pane, crate::workspace::main_area::pane_input_ops::PaneTextInput {
                    body: "manual input".into(),
                    intent: crate::workspace::main_area::pane_input_ops::PaneTextIntent::Command { submit: true },
                    task: None,
                }, window, cx));
            }
                (
                    id,
                    pane,
                    ws.active,
                    ws.active_runtime().focused_pane_id,
                    dispatched,
                    probe,
                )
            })
        })
        .unwrap();
    let result = dispatched.await;
    cx.run_until_parked();
    workspace.read_with(cx, |ws, cx| {
        assert_eq!(ws.active, active);
        assert_eq!(ws.active_runtime().focused_pane_id, focused);
        assert!(ws.main_area.pane(pane).is_some());
        let lifecycle = &cx.global::<GlobalTasks>().get(&id).unwrap().lifecycle;
        if release {
            assert!(result.is_err());
            assert!(!matches!(lifecycle, Lifecycle::Running { .. }));
            assert_eq!(probe.as_mut().unwrap().drain().0, 0);
        } else {
            assert!(result.is_ok(), "{result:?}");
            assert!(
                matches!(lifecycle, Lifecycle::Running { .. }),
                "{lifecycle:?}"
            );
            if let Some(probe) = &mut probe {
                assert_eq!(probe.drain().0, 1);
            }
        }
    });
}

#[gpui::test]
async fn a_terminal_task_reaches_its_pane_after_switching_lanes(cx: &mut TestAppContext) {
    dispatch_while_switching_lanes(TaskAgentSurface::Terminal, false, cx).await;
}

#[gpui::test]
async fn a_chat_task_reaches_its_pane_after_switching_lanes(cx: &mut TestAppContext) {
    dispatch_while_switching_lanes(TaskAgentSurface::AgentChat, false, cx).await;
}

#[gpui::test]
async fn a_task_unbound_during_preparation_never_reaches_the_chat(cx: &mut TestAppContext) {
    dispatch_while_switching_lanes(TaskAgentSurface::AgentChat, true, cx).await;
}

/// A started task is refused before `begin_task_start` returns; every other
/// refusal follows once earlier task commits have landed. None of them made
/// a lane or recorded a start.
#[gpui::test]
fn refusals_answer_before_anything_is_spawned(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    let gone = PathBuf::from("/nonexistent/daruda-lane");
    let (lanes, answers) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let lanes: usize = ws.projects.iter().map(|p| p.lanes.len()).sum();
                let here = ws.projects[0].uuid;
                let started = crate::runs::seed::task(
                    task(here),
                    Some((&SeedRun::chat(&gone), SeedEnd::Cancelled)),
                    cx,
                );
                let rx = ws.begin_task_start(&started, window, cx);
                assert_eq!(
                    rx.try_recv(),
                    Ok(Err(TaskStartError::NotBacklog)),
                    "a started task is refused at once"
                );
                let mut elsewhere = task(here);
                elsewhere.run_in = TaskRunIn::ExistingLane { path: gone.clone() };
                let cases = [
                    ("missing".to_owned(), TaskStartError::NotFound),
                    (
                        add(task(ProjectUuid::new()), cx),
                        TaskStartError::ProjectNotOpen,
                    ),
                    (add(task(here), cx), TaskStartError::NoGitRepo),
                    (
                        add(elsewhere, cx),
                        TaskStartError::LaneMissing { path: gone.clone() },
                    ),
                ];
                let answers: Vec<_> = cases
                    .into_iter()
                    .map(|(id, expected)| {
                        (id.clone(), ws.begin_task_start(&id, window, cx), expected)
                    })
                    .collect();
                (lanes, answers)
            })
        })
        .unwrap();
    cx.run_until_parked();
    for (id, rx, expected) in answers {
        assert_eq!(rx.try_recv(), Ok(Err(expected)));
        cx.update(|cx| {
            let lifecycle = cx
                .global::<crate::runs::GlobalTasks>()
                .get(&id)
                .map(|task| task.lifecycle.clone());
            assert!(
                lifecycle.is_none_or(|l| l == daruda_store::tasks::Lifecycle::Backlog),
                "a refusal records no start"
            );
        });
    }
    workspace.read_with(cx, |ws, _| {
        let after: usize = ws.projects.iter().map(|p| p.lanes.len()).sum();
        assert_eq!(after, lanes, "no refusal created a lane");
    });
}

#[gpui::test]
fn an_existing_lane_start_answers_with_where_it_runs(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    let (lane, rx) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let lane = ws.active_lane().unwrap().path.clone();
                let mut t = task(ws.projects[0].uuid);
                t.run_in = TaskRunIn::ExistingLane { path: lane.clone() };
                let id = add(t, cx);
                (lane, ws.begin_task_start(&id, window, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    let pane = workspace.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
    assert_eq!(
        rx.try_recv(),
        Ok(Ok(TaskStarted {
            worktree: lane,
            pane,
            surface: TaskAgentSurface::Terminal,
        }))
    );
}
