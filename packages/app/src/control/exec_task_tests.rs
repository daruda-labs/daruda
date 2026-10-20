//! The task commands: what they list, and what each refuses before asking.

use super::*;
use crate::control::approval::{ApprovalChoice, resolve_only_pending_for_test};
use crate::test_support::{ControlFixture, workspace_with_agent_chat};
use daruda_store::project::ProjectUuid;
use daruda_store::tasks::{Lifecycle, TaskRunIn};
use gpui::BorrowAppContext as _;

/// A bridge that can deliver an approval card, so a gated command waits.
fn pair_bridge(cx: &mut gpui::App) -> crate::telegram::global::TestOutbox {
    crate::settings_store::SettingsStore::init(cx);
    let outbound = crate::telegram::global::install_for_test(true, Some(42), cx);
    cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
        store.set_user_for_testing(daruda_config::Config {
            telegram: daruda_config::TelegramConfig {
                enabled: true,
                authorized_chat_id: Some(42),
                only_when_away: false,
            },
            ..daruda_config::Config::default()
        });
    });
    outbound
}

fn project_of(
    fixture: &ControlFixture,
    cx: &mut gpui::TestAppContext,
) -> (ProjectUuid, TaskProject) {
    fixture
        .workspace
        .read_with(cx, |ws, _| ws.control_task_projects().remove(0))
}

fn add_task(project: ProjectUuid, title: &str, cx: &mut gpui::TestAppContext) -> String {
    cx.update(|cx| crate::runs::seed::task(crate::runs::seed::spec(project, title), None, cx))
}

fn ready(dispatch: Dispatch) -> ControlOutcome {
    match dispatch {
        Dispatch::Ready(outcome) => outcome,
        Dispatch::Deferred { .. } => panic!("expected an answer before any card"),
    }
}

/// Tasks are listed by project, and one whose project no window has open
/// is listed last, with no project to name.
#[gpui::test]
async fn the_listing_groups_by_project_and_ends_with_closed_ones(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let (uuid, project) = project_of(&fixture, cx);
    let closed = add_task(ProjectUuid::new(), "closed", cx);
    let open = add_task(uuid, "open", cx);
    let outcome = cx.update(|cx| run(ResolvedCommand::TaskList, CommandOrigin::Orchestrator, cx));
    let Ok(ControlResult::TaskList { tasks }) = outcome else {
        panic!("{outcome:?}");
    };
    let rows: Vec<_> = tasks
        .iter()
        .map(|t| (t.task.clone(), t.project.clone()))
        .collect();
    assert_eq!(rows, vec![(open, Some(project)), (closed, None)]);
}

#[gpui::test]
async fn an_empty_title_refuses_before_the_card(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let (_, project) = project_of(&fixture, cx);
    let _outbound = cx.update(pair_bridge);
    cx.update(|cx| {
        let outcome = ready(run_gated(
            GatedCommand::TaskCreate {
                workspace: project.workspace,
                project: project.project,
                title: "  ".into(),
                prompt: "p".into(),
                worktree: None,
            },
            crate::control::exec::CommandOrigin::Orchestrator,
            cx,
        ));
        assert_eq!(outcome, Err(ControlError::TaskTitleEmpty));
        assert_eq!(crate::control::approval::waiting_count_for_test(cx), 0);
    });
}

/// An approved create adds a Backlog task to the project it named.
#[gpui::test]
async fn an_approved_create_adds_a_backlog_task_to_that_project(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let (uuid, project) = project_of(&fixture, cx);
    let _outbound = cx.update(pair_bridge);
    let dispatch = cx.update(|cx| {
        run_gated(
            GatedCommand::TaskCreate {
                workspace: project.workspace,
                project: project.project,
                title: " Fix login ".into(),
                prompt: "p".into(),
                worktree: None,
            },
            crate::control::exec::CommandOrigin::Orchestrator,
            cx,
        )
    });
    let Dispatch::Deferred { outcome: rx, .. } = dispatch else {
        panic!("a create asks first");
    };
    cx.update(|cx| resolve_only_pending_for_test(ApprovalChoice::Approved, cx));
    cx.run_until_parked();
    let Ok(Some(Ok(ControlResult::TaskCreated { task }))) = rx.recv().await else {
        panic!("not created");
    };
    cx.update(|cx| {
        let created = cx.global::<GlobalTasks>().get(&task).unwrap();
        assert_eq!(created.spec.project, uuid);
        assert_eq!(created.spec.title, "Fix login");
        assert_eq!(created.lifecycle, Lifecycle::Backlog);
        assert_eq!(created.spec.run_in, TaskRunIn::NewWorktree);
    });
}

/// A task whose project is not open has nowhere to run, so there is
/// nothing to ask about.
#[gpui::test]
async fn a_start_in_a_closed_project_refuses_before_the_card(cx: &mut gpui::TestAppContext) {
    let _fixture = workspace_with_agent_chat(cx);
    let task = add_task(ProjectUuid::new(), "closed", cx);
    let _outbound = cx.update(pair_bridge);
    cx.update(|cx| {
        let outcome = ready(run_gated(
            GatedCommand::TaskStart { task },
            crate::control::exec::CommandOrigin::Orchestrator,
            cx,
        ));
        assert_eq!(outcome, Err(ControlError::TaskProjectNotOpen));
        assert_eq!(crate::control::approval::waiting_count_for_test(cx), 0);
        assert!(crate::runs::RunsHost::run_scope(cx).is_empty());
    });
}

/// A refused start leaves the task where it was.
#[gpui::test]
async fn a_refused_start_runs_nothing(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let (uuid, _) = project_of(&fixture, cx);
    let task = add_task(uuid, "t", cx);
    let _outbound = cx.update(pair_bridge);
    let dispatch = cx.update(|cx| {
        run_gated(
            GatedCommand::TaskStart { task: task.clone() },
            crate::control::exec::CommandOrigin::Orchestrator,
            cx,
        )
    });
    let Dispatch::Deferred { outcome: rx, .. } = dispatch else {
        panic!("a start asks first");
    };
    cx.update(|cx| resolve_only_pending_for_test(ApprovalChoice::Refused, cx));
    cx.run_until_parked();
    assert_eq!(
        rx.recv().await,
        Ok(Some(Err(ControlError::ApprovalRefused)))
    );
    cx.update(|cx| {
        assert_eq!(
            cx.global::<GlobalTasks>().get(&task).unwrap().lifecycle,
            Lifecycle::Backlog
        );
    });
}

#[gpui::test]
async fn only_a_running_task_stops(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let (uuid, _) = project_of(&fixture, cx);
    let task = add_task(uuid, "t", cx);
    let outcome = cx.update(|cx| {
        ready(run_waiting(
            ResolvedCommand::TaskStop { task },
            crate::control::exec::CommandOrigin::Orchestrator,
            cx,
        ))
    });
    assert_eq!(outcome, Err(ControlError::TaskNotRunning));
    let outcome = cx.update(|cx| {
        ready(run_waiting(
            ResolvedCommand::TaskStop {
                task: "missing".into(),
            },
            crate::control::exec::CommandOrigin::Orchestrator,
            cx,
        ))
    });
    assert_eq!(outcome, Err(ControlError::TaskNotFound));
}

#[gpui::test]
async fn task_stop_answers_after_the_projection_changes(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let (project, _) = project_of(&fixture, cx);
    let task = cx.update(|cx| {
        let spec = crate::runs::seed::spec(project, "stop");
        let run = crate::runs::seed::SeedRun::chat(fixture.root());
        crate::runs::seed::task(spec, Some((&run, crate::runs::seed::SeedEnd::Running)), cx)
    });
    let immediate = cx.update(|cx| {
        run(
            ResolvedCommand::TaskStop { task: task.clone() },
            CommandOrigin::Orchestrator,
            cx,
        )
    });
    assert!(matches!(
        immediate,
        Err(ControlError::TaskNotRecorded { .. })
    ));
    let dispatch = cx.update(|cx| {
        run_waiting(
            ResolvedCommand::TaskStop { task: task.clone() },
            crate::control::exec::CommandOrigin::Orchestrator,
            cx,
        )
    });
    let Dispatch::Deferred { outcome, .. } = dispatch else {
        panic!("task_stop waits for its commit");
    };
    let answer = outcome.recv().await.unwrap().unwrap().unwrap();
    assert_eq!(answer, ControlResult::TaskStopped { task: task.clone() });
    cx.update(|cx| {
        assert!(matches!(
            cx.global::<GlobalTasks>().get(&task).unwrap().lifecycle,
            Lifecycle::Cancelled { .. }
        ))
    });
}

/// An approved start answers once the task runs: the worktree it runs in,
/// and the chat it runs as.
#[gpui::test]
async fn an_approved_start_answers_with_its_worktree_and_chat(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let _connections = cx.update(crate::workspace::Workspace::fake_agent_connections_for_test);
    let (workspace, lane) = fixture
        .workspace
        .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane()));
    let created = fixture.workspace.update(cx, |ws, cx| {
        ws.control_task_create(lane.project, "Here", "p".into(), Some(lane), cx)
            .expect("created")
    });
    cx.run_until_parked();
    let task = created.await.expect("recorded");
    let _outbound = cx.update(pair_bridge);
    let dispatch = cx.update(|cx| {
        run_gated(
            GatedCommand::TaskStart { task: task.clone() },
            crate::control::exec::CommandOrigin::Orchestrator,
            cx,
        )
    });
    let Dispatch::Deferred { outcome: rx, .. } = dispatch else {
        panic!("a start asks first");
    };
    cx.update(|cx| resolve_only_pending_for_test(ApprovalChoice::Approved, cx));
    cx.run_until_parked();
    let Ok(Some(Ok(ControlResult::TaskStarted {
        task: started,
        handle: run,
        lane: handle,
        chat: Some(chat),
    }))) = rx.recv().await
    else {
        panic!("not started");
    };
    cx.update(|cx| {
        assert_eq!(
            crate::runs::RunsHost::run_scope(cx),
            vec![crate::runs::result::RunKey::Task(
                daruda_store::tasks::ExecutionRef {
                    task_id: task.clone(),
                    execution_id: match &run {
                        daruda_control_types::runs::RunHandle::Task { execution_id, .. } =>
                            execution_id.clone(),
                        _ => panic!("task handle"),
                    }
                }
            )]
        );
    });
    assert_eq!(started, task);
    assert_eq!(handle, LaneHandle::new(workspace, lane));
    assert_eq!(chat.workspace, workspace);
    cx.update(|cx| {
        let task = cx.global::<GlobalTasks>().get(&task).unwrap();
        assert_ne!(task.lifecycle, Lifecycle::Backlog);
        assert!(task.execution.is_some(), "the run is bound to the task");
        assert_eq!(
            run.to_string(),
            format!("task:{}/{}", task.id(), task.execution.as_ref().unwrap().id)
        );
        assert_eq!(
            serde_json::to_value(task).unwrap()["started_by"],
            "orchestrator"
        );
    });
}

#[gpui::test]
async fn a_phone_task_start_records_phone_provenance(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let _connections = cx.update(crate::workspace::Workspace::fake_agent_connections_for_test);
    let lane = fixture
        .workspace
        .read_with(cx, |ws, _| ws.control_active_lane());
    let created = fixture.workspace.update(cx, |ws, cx| {
        ws.control_task_create(lane.project, "Phone", "p".into(), Some(lane), cx)
            .unwrap()
    });
    cx.run_until_parked();
    let task = created.await.unwrap();
    let pending = cx.update(|cx| {
        super::begin_task_start(task.clone(), CommandOrigin::Phone, true, cx).unwrap()
    });
    cx.run_until_parked();
    assert!(cx.update(|cx| pending.try_finish(cx)).unwrap().is_ok());
    cx.update(|cx| {
        assert_eq!(
            cx.global::<GlobalTasks>().get(&task).unwrap().started_by,
            Some(daruda_store::tasks::StartedBy::Phone)
        );
        assert!(crate::runs::RunsHost::run_scope(cx).is_empty());
    });
}
