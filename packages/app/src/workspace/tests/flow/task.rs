//! A task that runs as a flow, from its start to the task's end.

use super::*;
use crate::runs::GlobalTasks;
use daruda_store::tasks::{LifecycleKind, StartedBy, TaskFlow, TaskRunIn};

/// A command that writes the value it was given into the lane, and one that
/// waits to be stopped.
const FLOW: &str = "\
version: 1
inputs:
  pr:
    type: number
    required: true
nodes:
  - id: check
    kind: command
    run: \"echo {{input.pr}} > seen.txt\"
";

const SLOW: &str = "\
version: 1
inputs:
  topic:
    required: true
nodes:
  - id: wait
    kind: command
    run: \"sleep 30\"
";

/// A Backlog task in the fixture's lane that runs as `ship`, prompted
/// `prompt`.
fn flow_task(
    ws: &gpui::Entity<Workspace>,
    lane: &std::path::Path,
    prompt: &str,
    cx: &mut TestAppContext,
) -> String {
    ws.update(cx, |ws, cx| {
        let mut spec = crate::runs::seed::spec(ws.projects[0].uuid, "Review");
        spec.prompt = prompt.into();
        spec.run_in = TaskRunIn::ExistingLane {
            path: lane.to_path_buf(),
        };
        spec.flow = Some(TaskFlow {
            name: "ship".into(),
            profile: None,
            input: None,
        });
        crate::runs::seed::task(spec, None, cx)
    })
}

fn start(
    ws: &gpui::Entity<Workspace>,
    wh: gpui::WindowHandle<gpui_component::Root>,
    id: &str,
    cx: &mut TestAppContext,
) -> smol::channel::Receiver<crate::workspace::right_dock::task_start::TaskStartOutcome> {
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.begin_task_start(id, StartedBy::Orchestrator, window, cx)
        })
    })
    .expect("window is live")
}

fn kind(id: &str, cx: &mut TestAppContext) -> LifecycleKind {
    cx.update(|cx| {
        cx.global::<GlobalTasks>()
            .get(id)
            .expect("the task")
            .lifecycle
            .kind()
    })
}

/// The prompt is the flow's input, verbatim; the run's end is the task's.
#[gpui::test]
async fn a_flow_task_runs_its_prompt_through_the_flow(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (lane, ws, _path, wh) = workspace_with_a_flow(cx, FLOW);
    let id = flow_task(&ws, lane.path(), "1234", cx);
    let answer = start(&ws, wh, &id, cx);
    assert!(
        wait_for_run(cx, |cx| kind(&id, cx) == LifecycleKind::Done).await,
        "the task ended with its run: {:?}",
        kind(&id, cx)
    );
    let started = answer.try_recv().expect("answered").expect("started");
    assert_eq!(started.chat(), None, "a flow run has no one chat");
    assert_eq!(
        std::fs::read_to_string(lane.path().join("seen.txt")).unwrap(),
        "1234\n"
    );
    cx.update(|cx| {
        let task = cx.global::<GlobalTasks>().get(&id).unwrap();
        let run = task.ended_executions.last().unwrap();
        assert!(
            run.execution
                .as_ref()
                .is_some_and(|run| run.source.flow_run_id().is_some()),
            "the run is recorded as the flow's"
        );
    });
}

/// A prompt the flow's input will not take refuses the start before
/// anything is recorded: the task stays in Backlog, to be fixed.
#[gpui::test]
async fn a_prompt_the_input_refuses_leaves_the_task_in_backlog(cx: &mut TestAppContext) {
    let (lane, ws, _path, wh) = workspace_with_a_flow(cx, FLOW);
    let id = flow_task(&ws, lane.path(), "not a number", cx);
    let answer = start(&ws, wh, &id, cx);
    cx.run_until_parked();
    assert!(matches!(
        answer.try_recv(),
        Ok(Err(
            crate::workspace::right_dock::task_start::TaskStartError::FlowRefused { .. }
        ))
    ));
    assert_eq!(kind(&id, cx), LifecycleKind::Backlog);
    assert!(!run_is_held(cx, lane.path()));
}

/// Cancelling the task stops its run, and the run's end leaves the task
/// cancelled rather than settling it again.
#[gpui::test]
async fn cancelling_a_flow_task_stops_its_run(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (lane, ws, _path, wh) = workspace_with_a_flow(cx, SLOW);
    let id = flow_task(&ws, lane.path(), "anything", cx);
    let _answer = start(&ws, wh, &id, cx);
    assert!(
        wait_for_run(cx, |cx| kind(&id, cx) == LifecycleKind::Running).await,
        "the run started"
    );
    let exec = cx.update(|cx| {
        cx.global::<GlobalTasks>()
            .get(&id)
            .unwrap()
            .current_exec()
            .cloned()
            .unwrap()
    });
    let cancelled = ws.update(cx, |ws, cx| ws.cancel_execution(exec, cx));
    cancelled.await.expect("the cancel is recorded");
    assert!(
        wait_for_run(cx, |cx| !run_is_held(cx, lane.path())).await,
        "the run stopped"
    );
    assert_eq!(kind(&id, cx), LifecycleKind::Cancelled);
}

/// Stopping the run from the Flows panel or the phone ends the task too.
#[gpui::test]
async fn stopping_a_flow_tasks_run_stops_the_task(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (lane, ws, _path, wh) = workspace_with_a_flow(cx, SLOW);
    let id = flow_task(&ws, lane.path(), "anything", cx);
    let _answer = start(&ws, wh, &id, cx);
    assert!(wait_for_run(cx, |cx| kind(&id, cx) == LifecycleKind::Running).await);
    cx.update(|cx| crate::runs::RunsHost::stop_flow(lane.path(), cx));
    assert!(
        wait_for_run(cx, |cx| kind(&id, cx) == LifecycleKind::Interrupted).await,
        "the task was stopped with its run: {:?}",
        kind(&id, cx)
    );
}

/// A task with no worktree of its own gets one, with nothing open in it: the
/// run is what works there. The flow is read from the lane it branches from.
#[gpui::test]
async fn a_flow_task_runs_in_a_new_worktree_of_its_own(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }
    cx.executor().allow_parking();
    let repo = tempfile::tempdir().unwrap();
    super::super::tasks::git_repo(repo.path());
    let flows = crate::workspace::flow_paths::flows_dir(repo.path());
    std::fs::create_dir_all(&flows).unwrap();
    std::fs::write(flows.join("ship.yaml"), FLOW).unwrap();
    let built = build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(repo.path())),
    );
    let (wh, ws) = registered(cx, built);
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();
    let id = ws.update(cx, |ws, cx| {
        let mut spec = crate::runs::seed::spec(ws.projects[0].uuid, "Review");
        spec.prompt = "77".into();
        spec.flow = Some(TaskFlow {
            name: "ship.yaml".into(),
            profile: None,
            input: Some("pr".into()),
        });
        crate::runs::seed::task(spec, None, cx)
    });
    let answer = start(&ws, wh, &id, cx);
    assert!(
        wait_for_run(cx, |cx| kind(&id, cx) == LifecycleKind::Done).await,
        "{:?}",
        kind(&id, cx)
    );
    let started = answer.try_recv().expect("answered").expect("started");
    assert_ne!(started.worktree, repo.path());
    assert_eq!(
        std::fs::read_to_string(started.worktree.join("seen.txt")).unwrap(),
        "77\n"
    );
    ws.read_with(cx, |ws, _| {
        let lane = ws.lane_ref_at(&started.worktree).expect("lane registered");
        let runtime = ws.main_area.runtimes.get(&lane);
        assert!(
            runtime.is_none_or(|runtime| runtime.panes.is_empty()),
            "nothing is opened in a flow task's worktree"
        );
    });
}

/// A flow that fails its own static checks is refused before the start is
/// recorded: the task stays in Backlog and nothing runs.
#[gpui::test]
async fn an_invalid_flow_leaves_the_task_in_backlog(cx: &mut TestAppContext) {
    let (lane, ws, _path, wh) = workspace_with_a_flow(
        cx,
        "version: 1\ninputs:\n  pr: { required: true }\nnodes:\n  - id: a\n    kind: command\n    deps: [gone]\n    run: \"exit 0\"\n",
    );
    let id = flow_task(&ws, lane.path(), "1", cx);
    let answer = start(&ws, wh, &id, cx);
    cx.run_until_parked();
    assert!(matches!(
        answer.try_recv(),
        Ok(Err(
            crate::workspace::right_dock::task_start::TaskStartError::FlowRefused { .. }
        ))
    ));
    assert_eq!(kind(&id, cx), LifecycleKind::Backlog);
    assert!(!run_is_held(cx, lane.path()));
}

/// An approval always says how freely the flow acts: a flow of commands has
/// no agents to give a permission mode, and one that is not found says so.
#[gpui::test]
fn a_flow_tasks_approval_names_what_it_runs(cx: &mut TestAppContext) {
    use crate::surface::strings::control as s;
    let (lane, ws, _path, _wh) = workspace_with_a_flow(cx, FLOW);
    ws.read_with(cx, |ws, _| {
        let run_in = TaskRunIn::ExistingLane {
            path: lane.path().to_path_buf(),
        };
        let flow = |name: &str| TaskFlow {
            name: name.into(),
            profile: None,
            input: None,
        };
        let project = ws.projects[0].uuid;
        assert_eq!(
            ws.task_flow_approval(project, &run_in, None, &flow("ship")),
            s::approval_task_as_flow("ship.yaml", s::approval_flow_no_agents())
        );
        assert_eq!(
            ws.task_flow_approval(project, &run_in, None, &flow("gone")),
            s::approval_task_as_flow_unresolved("gone")
        );
    });
}
