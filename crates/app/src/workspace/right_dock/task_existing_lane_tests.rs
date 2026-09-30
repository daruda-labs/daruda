//! A task pointed at an existing lane runs there without creating one.

use std::path::{Path, PathBuf};

use crate::agent::tasks_global::GlobalTasks;
use crate::workspace::Workspace;
use crate::workspace::tests::build_workspace_with;
use daruda_store::tasks::{Task, TaskAgentSurface, TaskRunIn, TaskState};
use gpui::{AppContext as _, BorrowAppContext as _, Context, TestAppContext};

fn existing_lane_task(
    path: &Path,
    surface: TaskAgentSurface,
    cx: &mut Context<Workspace>,
) -> String {
    let mut task = Task::new("Fix it".into(), "Prompt".into(), None);
    task.agent_surface = surface;
    task.run_in = TaskRunIn::ExistingLane {
        path: path.to_path_buf(),
    };
    let id = task.id.clone();
    cx.update_global::<GlobalTasks, _>(|tasks, _| {
        tasks.add(task);
    });
    id
}

fn lane_count(ws: &Workspace) -> usize {
    ws.projects.iter().map(|p| p.lanes.len()).sum()
}

fn state(id: &str, cx: &Context<Workspace>) -> TaskState {
    cx.global::<GlobalTasks>().get(id).unwrap().state.clone()
}

#[gpui::test]
fn existing_lane_start_opens_a_tab_there_instead_of_a_worktree(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let lane_path = ws.active_lane().unwrap().path.clone();
            let lanes = lane_count(ws);
            let tabs = ws.active_runtime().tabs.len();
            let id = existing_lane_task(&lane_path, TaskAgentSurface::AgentChat, cx);

            ws.start_task(&id, window, cx);

            assert_eq!(lane_count(ws), lanes, "no lane is created");
            assert_eq!(ws.active_runtime().tabs.len(), tabs + 1);
            assert_eq!(
                state(&id, cx),
                TaskState::Running {
                    worktree_path: lane_path.clone()
                }
            );
        })
    })
    .unwrap();
}

#[gpui::test]
fn existing_lane_terminal_start_runs_with_the_prompt_file_in_that_lane(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let lane_path = ws.active_lane().unwrap().path.clone();
            let lanes = lane_count(ws);
            let tabs = ws.active_runtime().tabs.len();
            let id = existing_lane_task(&lane_path, TaskAgentSurface::Terminal, cx);

            ws.start_task(&id, window, cx);

            assert_eq!(lane_count(ws), lanes);
            assert_eq!(ws.active_runtime().tabs.len(), tabs + 1);
            let task = cx.global::<GlobalTasks>().get(&id).unwrap().clone();
            let prompt = daruda_store::tasks::prompt_file_path(&task, &lane_path);
            assert!(prompt.ends_with(format!("task-{id}.md")));
            let body = std::fs::read_to_string(prompt).unwrap();
            assert!(body.starts_with("Task: \"Fix it\" ("));
            assert_eq!(
                state(&id, cx),
                TaskState::Running {
                    worktree_path: lane_path
                }
            );
        })
    })
    .unwrap();
}

#[gpui::test]
fn existing_lane_start_finds_a_lane_in_an_inactive_project(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let owner = ws.active;
            let lane_path = ws.active_lane().unwrap().path.clone();
            let other = tempfile::tempdir().unwrap();
            ws.add_project(other.path().to_path_buf(), window, cx);
            assert_ne!(ws.active, owner);
            let id = existing_lane_task(&lane_path, TaskAgentSurface::AgentChat, cx);

            ws.start_task(&id, window, cx);

            assert_eq!(ws.active, owner);
            assert!(matches!(state(&id, cx), TaskState::Running { .. }));
        })
    })
    .unwrap();
}

#[gpui::test]
fn existing_lane_start_keeps_the_task_in_backlog_when_the_lane_is_gone(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let lanes = lane_count(ws);
            let tabs = ws.active_runtime().tabs.len();
            let id = existing_lane_task(
                &PathBuf::from("/nonexistent/daruda-lane"),
                TaskAgentSurface::AgentChat,
                cx,
            );

            ws.start_task(&id, window, cx);

            assert_eq!(state(&id, cx), TaskState::Backlog);
            assert_eq!(lane_count(ws), lanes);
            assert_eq!(ws.active_runtime().tabs.len(), tabs);
            assert_eq!(
                ws.error_history.first().map(|r| r.title.clone()),
                Some(crate::surface::strings::error_task_lane_missing()),
            );
        })
    })
    .unwrap();
}

fn finished_task(lane: &Path, cx: &mut Context<Workspace>) -> String {
    let mut task = Task::new("Again".into(), "Prompt".into(), None);
    task.agent_surface = TaskAgentSurface::AgentChat;
    task.state = TaskState::Error {
        worktree_path: lane.to_path_buf(),
        message: "failed".into(),
    };
    task.finished_at = Some(chrono::Utc::now());
    let id = task.id.clone();
    cx.update_global::<GlobalTasks, _>(|tasks, _| {
        tasks.add(task);
    });
    id
}

fn run_in(id: &str, cx: &Context<Workspace>) -> TaskRunIn {
    cx.global::<GlobalTasks>().get(id).unwrap().run_in.clone()
}

/// Its branch is checked out in the lane its first run created, so running
/// it again as a new worktree would fail in git; it resumes in that lane.
#[gpui::test]
fn retry_runs_again_in_the_lane_the_task_created(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let lane_path = ws.active_lane().unwrap().path.clone();
            let lanes = lane_count(ws);
            let id = finished_task(&lane_path, cx);

            ws.retry_task(&id, window, cx);

            assert_eq!(
                run_in(&id, cx),
                TaskRunIn::ExistingLane {
                    path: lane_path.clone()
                }
            );
            assert_eq!(lane_count(ws), lanes);
            assert_eq!(
                state(&id, cx),
                TaskState::Running {
                    worktree_path: lane_path
                }
            );
            let task = cx.global::<GlobalTasks>().get(&id).unwrap();
            assert_eq!(task.finished_at, None, "the old run's end is cleared");
        })
    })
    .unwrap();
}

#[gpui::test]
fn reopen_keeps_a_new_worktree_task_whose_lane_is_gone(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            let lane_path = ws.active_lane().unwrap().path.clone();
            let kept = finished_task(&lane_path, cx);
            let gone = finished_task(&PathBuf::from("/nonexistent/daruda-lane"), cx);

            ws.reopen_task(&kept, cx);
            ws.reopen_task(&gone, cx);

            assert_eq!(
                run_in(&kept, cx),
                TaskRunIn::ExistingLane { path: lane_path }
            );
            assert_eq!(run_in(&gone, cx), TaskRunIn::NewWorktree);
            assert_eq!(state(&gone, cx), TaskState::Backlog);
        })
    })
    .unwrap();
}

#[gpui::test]
fn existing_lane_start_reports_a_registered_lane_whose_directory_is_gone(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, window, cx| {
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

            assert_eq!(state(&id, cx), TaskState::Backlog);
            assert_eq!(ws.error_history.len(), 1, "reported once");
        })
    })
    .unwrap();
}
