//! A task pointed at an existing lane runs there without creating one.

use std::path::{Path, PathBuf};

use crate::runs::GlobalTasks;
use crate::runs::seed::{SeedEnd, SeedRun};
use crate::workspace::Workspace;
use crate::workspace::tests::build_workspace_with;
use daruda_store::tasks::{Lifecycle, TaskAgentSurface, TaskRunIn};
use gpui::{AppContext as _, Context, TestAppContext};

fn existing_lane_task(
    path: &Path,
    surface: TaskAgentSurface,
    cx: &mut Context<Workspace>,
) -> String {
    let mut task = crate::runs::seed::spec(daruda_store::project::ProjectUuid::default(), "Fix it");
    task.agent_surface = surface;
    task.run_in = TaskRunIn::ExistingLane {
        path: path.to_path_buf(),
    };
    crate::runs::seed::task(task, None, cx)
}

fn lane_count(ws: &Workspace) -> usize {
    ws.projects.iter().map(|p| p.lanes.len()).sum()
}

fn state(id: &str, cx: &mut TestAppContext) -> Lifecycle {
    cx.update(|cx| {
        cx.global::<GlobalTasks>()
            .get(id)
            .unwrap()
            .lifecycle
            .clone()
    })
}

fn running_in(state: &Lifecycle) -> Option<&Path> {
    match state {
        Lifecycle::Running { worktree, .. } => Some(worktree),
        _ => None,
    }
}

/// The lane prepared for the run, even if its prompt never reached an adapter.
fn prepared_in(state: &Lifecycle) -> Option<&Path> {
    match state {
        Lifecycle::Undelivered {
            worktree: Some(worktree),
            ..
        }
        | Lifecycle::Dispatching { worktree, .. } => Some(worktree),
        Lifecycle::Error {
            worktree: Some(worktree),
            cause: daruda_store::tasks::machine::ErrorCause::SessionFailed,
            ..
        } => Some(worktree),
        other => running_in(other),
    }
}

fn project_workspace(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    (root, window, workspace)
}

#[gpui::test]
fn existing_lane_start_opens_a_tab_there_instead_of_a_worktree(cx: &mut TestAppContext) {
    let (_root, window, workspace) = project_workspace(cx);
    let (id, lane_path, lanes, tabs) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let lane_path = ws.active_lane().unwrap().path.clone();
                let lanes = lane_count(ws);
                let tabs = ws.active_runtime().tabs.len();
                let id = existing_lane_task(&lane_path, TaskAgentSurface::AgentChat, cx);
                ws.start_task(&id, window, cx);
                (id, lane_path, lanes, tabs)
            })
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(lane_count(ws), lanes, "no lane is created");
        assert_eq!(ws.active_runtime().tabs.len(), tabs + 1);
    });
    assert_eq!(prepared_in(&state(&id, cx)), Some(lane_path.as_path()));
}

#[gpui::test]
fn remote_lanes_are_refused_before_dispatch_requested(cx: &mut TestAppContext) {
    let (_root, window, workspace) = project_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let path = ws.active_lane().unwrap().path.clone();
            ws.set_lane_session_host(
                ws.active,
                daruda_store::project::LaneSessionHost::Ssh {
                    target: "test-host".into(),
                    session_path: "/work".into(),
                    registry_id: None,
                },
                cx,
            );
            let id = existing_lane_task(&path, TaskAgentSurface::AgentChat, cx);
            let before = crate::runs::RunsHost::sequence_for_test(cx);
            let result = ws
                .begin_task_start(&id, daruda_store::tasks::StartedBy::Desktop, window, cx)
                .try_recv();
            assert_eq!(result, Ok(Err(super::TaskStartError::RemoteLane)));
            assert_eq!(crate::runs::RunsHost::sequence_for_test(cx), before);
            assert_eq!(
                cx.global::<GlobalTasks>().get(&id).unwrap().lifecycle,
                Lifecycle::Backlog
            );
        })
    })
    .unwrap();
}

#[gpui::test]
fn existing_lane_terminal_start_runs_with_the_prompt_file_in_that_lane(cx: &mut TestAppContext) {
    let (_root, window, workspace) = project_workspace(cx);
    let (id, lane_path, lanes, tabs) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let lane_path = ws.active_lane().unwrap().path.clone();
                let lanes = lane_count(ws);
                let tabs = ws.active_runtime().tabs.len();
                let id = existing_lane_task(&lane_path, TaskAgentSurface::Terminal, cx);
                ws.start_task(&id, window, cx);
                (id, lane_path, lanes, tabs)
            })
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(lane_count(ws), lanes);
        assert_eq!(ws.active_runtime().tabs.len(), tabs + 1);
    });
    let spec = cx.update(|cx| cx.global::<GlobalTasks>().get(&id).unwrap().spec.clone());
    let prompt = daruda_store::tasks::prompt_file_path(&spec, &lane_path);
    assert!(prompt.ends_with(format!("task-{id}.md")));
    let body = std::fs::read_to_string(prompt).unwrap();
    assert!(body.starts_with("Task: \"Fix it\" ("));
    assert_eq!(running_in(&state(&id, cx)), Some(lane_path.as_path()));
}

#[gpui::test]
fn existing_lane_start_finds_a_lane_in_an_inactive_project(cx: &mut TestAppContext) {
    let (_root, window, workspace) = project_workspace(cx);
    let other = tempfile::tempdir().unwrap();
    let (id, owner) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let owner = ws.active;
                let lane_path = ws.active_lane().unwrap().path.clone();
                ws.add_project(other.path().to_path_buf(), window, cx);
                assert_ne!(ws.active, owner);
                let id = existing_lane_task(&lane_path, TaskAgentSurface::AgentChat, cx);
                ws.start_task(&id, window, cx);
                (id, owner)
            })
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| assert_eq!(ws.active, owner));
    assert!(prepared_in(&state(&id, cx)).is_some());
}

#[gpui::test]
fn existing_lane_start_keeps_the_task_in_backlog_when_the_lane_is_gone(cx: &mut TestAppContext) {
    let (_root, window, workspace) = project_workspace(cx);
    let (id, lanes, tabs) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let lanes = lane_count(ws);
                let tabs = ws.active_runtime().tabs.len();
                let id = existing_lane_task(
                    &PathBuf::from("/nonexistent/daruda-lane"),
                    TaskAgentSurface::AgentChat,
                    cx,
                );
                ws.start_task(&id, window, cx);
                (id, lanes, tabs)
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(state(&id, cx), Lifecycle::Backlog);
    workspace.read_with(cx, |ws, _| {
        assert_eq!(lane_count(ws), lanes);
        assert_eq!(ws.active_runtime().tabs.len(), tabs);
        assert_eq!(
            ws.error_history.first().map(|r| r.title.clone()),
            Some(crate::surface::strings::error::task_lane_missing()),
        );
    });
}

/// A task whose run in `lane` failed.
fn finished_task(lane: &Path, cx: &mut Context<Workspace>) -> String {
    let mut task = crate::runs::seed::spec(daruda_store::project::ProjectUuid::default(), "Again");
    task.agent_surface = TaskAgentSurface::AgentChat;
    let run = SeedRun::chat(lane);
    crate::runs::seed::task(task, Some((&run, SeedEnd::Errored)), cx)
}

fn run_in(id: &str, cx: &mut TestAppContext) -> TaskRunIn {
    cx.update(|cx| {
        cx.global::<GlobalTasks>()
            .get(id)
            .unwrap()
            .spec
            .run_in
            .clone()
    })
}

/// Its branch is checked out in the lane its first run created, so running
/// it again as a new worktree would fail in git; it resumes in that lane.
#[gpui::test]
fn retry_runs_again_in_the_lane_the_task_created(cx: &mut TestAppContext) {
    let (_root, window, workspace) = project_workspace(cx);
    let (id, lane_path, lanes, previous_end) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let lane_path = ws.active_lane().unwrap().path.clone();
                let lanes = lane_count(ws);
                let id = finished_task(&lane_path, cx);
                let previous_end = cx.global::<GlobalTasks>().get(&id).unwrap().finished_at;
                ws.retry_task(&id, window, cx);
                (id, lane_path, lanes, previous_end)
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        run_in(&id, cx),
        TaskRunIn::ExistingLane {
            path: lane_path.clone()
        }
    );
    workspace.read_with(cx, |ws, _| assert_eq!(lane_count(ws), lanes));
    assert_eq!(prepared_in(&state(&id, cx)), Some(lane_path.as_path()));
    cx.update(|cx| {
        let task = cx.global::<GlobalTasks>().get(&id).unwrap();
        assert_ne!(
            task.finished_at, previous_end,
            "the old run's end is cleared"
        );
        assert_eq!(task.finished_at.is_some(), task.lifecycle.is_terminal());
    });
}

#[gpui::test]
fn reopen_keeps_a_new_worktree_task_whose_lane_is_gone(cx: &mut TestAppContext) {
    let (_root, window, workspace) = project_workspace(cx);
    let (kept, gone, lane_path) = cx
        .update_window(window.into(), |_, _window, cx| {
            workspace.update(cx, |ws, cx| {
                let lane_path = ws.active_lane().unwrap().path.clone();
                let kept = finished_task(&lane_path, cx);
                let gone = finished_task(&PathBuf::from("/nonexistent/daruda-lane"), cx);
                ws.reopen_task(&kept, cx);
                ws.reopen_task(&gone, cx);
                (kept, gone, lane_path)
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        run_in(&kept, cx),
        TaskRunIn::ExistingLane { path: lane_path }
    );
    assert_eq!(run_in(&gone, cx), TaskRunIn::NewWorktree);
    assert_eq!(state(&gone, cx), Lifecycle::Backlog);
}

#[gpui::test]
fn existing_lane_start_reports_a_registered_lane_whose_directory_is_gone(cx: &mut TestAppContext) {
    let (_root, window, workspace) = project_workspace(cx);
    let id = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let gone = std::env::temp_dir().join("daruda-registered-but-gone");
                let project = ws.active.project;
                let lane = ws.alloc_id();
                ws.project_for_mut(project)
                    .unwrap()
                    .lanes
                    .push(crate::lane::Lane::default_for_project(lane, gone.clone()));
                let id = existing_lane_task(&gone, TaskAgentSurface::AgentChat, cx);
                ws.start_task(&id, window, cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(state(&id, cx), Lifecycle::Backlog);
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.error_history.len(), 1, "reported once")
    });
}

/// A Retry whose Start stops early (here: no git repo) still leaves the task
/// in Backlog in the log without creating a legacy task file.
#[gpui::test]
fn retry_that_cannot_start_persists_the_backlog_state(cx: &mut TestAppContext) {
    let (window, workspace) = crate::workspace::tests::build_workspace(cx);
    let data_dir = workspace.read_with(cx, |ws, _| ws.data_dir.clone());
    cx.update(crate::runs::RunsHost::seed_for_test);
    let id = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let id = finished_task(&PathBuf::from("/nonexistent/daruda-lane"), cx);
                ws.retry_task(&id, window, cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(state(&id, cx), Lifecycle::Backlog);
    assert!(!daruda_store::tasks::tasks_path_in(&data_dir).exists());
}

/// The person at the window keeps what they were looking at — the active
/// lane and its focused pane — while the started task's chat is made in its
/// own lane and its session still connects.
#[gpui::test]
fn a_phone_start_runs_in_its_lane_without_taking_the_window(cx: &mut TestAppContext) {
    let (_root, window, workspace) = project_workspace(cx);
    let other = tempfile::tempdir().unwrap();
    let probes = cx.update(Workspace::fake_agent_connections_for_test);
    let (id, owner, here, focused, owner_tabs) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let owner = ws.active;
                let lane_path = ws.active_lane().unwrap().path.clone();
                let owner_tabs = ws.active_runtime().tabs.len();
                ws.add_project(other.path().to_path_buf(), window, cx);
                let here = ws.active;
                assert_ne!(here, owner);
                let focused = ws.active_runtime().focused_pane_id;
                let id = existing_lane_task(&lane_path, TaskAgentSurface::AgentChat, cx);
                drop(ws.begin_task_start(&id, daruda_store::tasks::StartedBy::Phone, window, cx));
                (id, owner, here, focused, owner_tabs)
            })
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |ws, cx| {
        assert_eq!(ws.active, here, "the active lane stays");
        assert_eq!(ws.active_runtime().focused_pane_id, focused);
        let rt = ws.main_area.runtimes.get(&owner).unwrap();
        assert_eq!(
            rt.tabs.len(),
            owner_tabs + 1,
            "the task's tab is in its lane"
        );
        let pane = rt.tabs.last().unwrap().last_focused_pane;
        assert_eq!(
            ws.agent_chat_view(pane).unwrap().read(cx).status(),
            &daruda_chat::view::AgentSessionStatus::Connected,
            "its session connects without being shown"
        );
    });
    assert_eq!(probes.borrow().len(), 1);
    assert!(
        running_in(&state(&id, cx)).is_some(),
        "{:?}",
        state(&id, cx)
    );
}

/// Started in the lane the orchestrator's tab is shown in, the task's tab
/// joins that lane behind it: the person keeps talking to the orchestrator.
#[gpui::test]
fn an_orchestrator_start_in_the_shown_lane_keeps_its_tab_in_front(cx: &mut TestAppContext) {
    let (_root, window, workspace) = project_workspace(cx);
    let probes = cx.update(Workspace::fake_agent_connections_for_test);
    let (id, here, orchestrator, tabs) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let orchestrator = ws
                    .seed_orchestrator_chat_pane_unrevealed_for_test(
                        ws.mirrors.agents[0].id.clone(),
                        std::env::temp_dir(),
                        daruda_store::accounts::AccountSelection::SystemDefault,
                        None,
                        window,
                        cx,
                    )
                    .unwrap();
                assert!(ws.show_orchestrator_tab(window, cx));
                let lane_path = ws.active_lane().unwrap().path.clone();
                let tabs = ws.active_runtime().tabs.len();
                let id = existing_lane_task(&lane_path, TaskAgentSurface::AgentChat, cx);
                drop(ws.begin_task_start(
                    &id,
                    daruda_store::tasks::StartedBy::Orchestrator,
                    window,
                    cx,
                ));
                (id, ws.active, orchestrator, tabs)
            })
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active, here);
        let rt = ws.active_runtime();
        assert_eq!(rt.tabs.len(), tabs + 1);
        assert!(
            ws.is_orchestrator_tab(&rt.tabs[rt.active_tab_index]),
            "the orchestrator's tab stays in front"
        );
        assert_eq!(rt.focused_pane_id, orchestrator);
    });
    assert_eq!(
        probes.borrow().len(),
        1,
        "only the task's session connected"
    );
    assert!(
        running_in(&state(&id, cx)).is_some(),
        "{:?}",
        state(&id, cx)
    );
}
