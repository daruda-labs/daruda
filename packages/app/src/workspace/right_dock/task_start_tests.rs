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
