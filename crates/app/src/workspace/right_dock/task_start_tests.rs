//! A start answers with its outcome, and a refusal answers before git runs.

use std::path::PathBuf;

use super::{TaskStartError, TaskStarted};
use crate::agent::tasks_global::GlobalTasks;
use crate::workspace::Workspace;
use crate::workspace::tests::build_workspace_with;
use daruda_store::project::ProjectUuid;
use daruda_store::tasks::{Task, TaskAgentSurface, TaskRunIn, TaskState};
use gpui::{AppContext as _, BorrowAppContext as _, Context, TestAppContext};

fn add(task: Task, cx: &mut Context<Workspace>) -> String {
    let id = task.id.clone();
    cx.update_global::<GlobalTasks, _>(|g, _| {
        g.add(task);
    });
    id
}

fn task(project: ProjectUuid) -> Task {
    let mut task = Task::new(project, "T".into(), "P".into(), None);
    task.agent_surface = TaskAgentSurface::Terminal;
    task
}

/// Each refusal is on the channel before `begin_task_start` returns, so a
/// caller can answer in the same turn — and none of them made a lane.
#[gpui::test]
fn refusals_answer_before_anything_is_spawned(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let lanes: usize = ws.projects.iter().map(|p| p.lanes.len()).sum();
            let here = ws.projects[0].uuid;
            let gone = PathBuf::from("/nonexistent/daruda-lane");

            let mut started = task(here);
            started.state = TaskState::Cancelled {
                worktree_path: gone.clone(),
            };
            let mut elsewhere = task(here);
            elsewhere.run_in = TaskRunIn::ExistingLane { path: gone.clone() };
            let cases = [
                ("missing".to_owned(), TaskStartError::NotFound),
                (add(started, cx), TaskStartError::NotBacklog),
                (
                    add(task(ProjectUuid::new()), cx),
                    TaskStartError::ProjectNotOpen,
                ),
                (add(task(here), cx), TaskStartError::NoGitRepo),
                (
                    add(elsewhere, cx),
                    TaskStartError::LaneMissing { path: gone },
                ),
            ];
            for (id, expected) in cases {
                let rx = ws.begin_task_start(&id, window, cx);
                assert_eq!(rx.try_recv(), Ok(Err(expected)));
            }
            let after: usize = ws.projects.iter().map(|p| p.lanes.len()).sum();
            assert_eq!(after, lanes, "no refusal created a lane");
        })
    })
    .unwrap();
}

#[gpui::test]
fn an_existing_lane_start_answers_with_where_it_runs(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let lane = ws.active_lane().unwrap().path.clone();
            let mut t = task(ws.projects[0].uuid);
            t.run_in = TaskRunIn::ExistingLane { path: lane.clone() };
            let id = add(t, cx);
            let rx = ws.begin_task_start(&id, window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            assert_eq!(
                rx.try_recv(),
                Ok(Ok(TaskStarted {
                    worktree: lane,
                    pane,
                    surface: TaskAgentSurface::Terminal,
                }))
            );
        })
    })
    .unwrap();
}
