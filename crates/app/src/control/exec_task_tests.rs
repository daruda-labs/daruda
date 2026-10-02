//! The task commands: what they list, and what each refuses before asking.

use super::*;
use crate::control::approval::{ApprovalChoice, resolve_only_pending_for_test};
use crate::test_support::{ControlFixture, workspace_with_agent_chat};
use daruda_store::project::ProjectUuid;
use daruda_store::tasks::{Task, TaskRunIn, TaskState};
use gpui::BorrowAppContext as _;

/// A bridge that can deliver an approval card, so a gated command waits.
fn pair_bridge(cx: &mut gpui::App) {
    crate::settings_store::SettingsStore::init(cx);
    crate::telegram::global::install_for_test(true, Some(42), cx);
    cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
        store.set_user_for_testing(daruda_config::Config {
            telegram: daruda_config::TelegramConfig {
                enabled: true,
                authorized_chat_id: Some(42),
                ..Default::default()
            },
            ..daruda_config::Config::default()
        });
    });
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
    cx.update(|cx| {
        let task = Task::new(project, title.into(), "prompt".into(), None);
        let id = task.id.clone();
        cx.update_global::<GlobalTasks, _>(|g, _| {
            g.add(task);
        });
        id
    })
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
    let outcome = cx.update(|cx| run(ResolvedCommand::TaskList, cx));
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
    cx.update(|cx| {
        pair_bridge(cx);
        let outcome = ready(run_gated(
            GatedCommand::TaskCreate {
                workspace: project.workspace,
                project: project.project,
                title: "  ".into(),
                prompt: "p".into(),
                worktree: None,
            },
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
    let dispatch = cx.update(|cx| {
        pair_bridge(cx);
        run_gated(
            GatedCommand::TaskCreate {
                workspace: project.workspace,
                project: project.project,
                title: " Fix login ".into(),
                prompt: "p".into(),
                worktree: None,
            },
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
        assert_eq!(created.project, uuid);
        assert_eq!(created.title, "Fix login");
        assert_eq!(created.state, TaskState::Backlog);
        assert_eq!(created.run_in, TaskRunIn::NewWorktree);
    });
}

/// A task whose project is not open has nowhere to run, so there is
/// nothing to ask about.
#[gpui::test]
async fn a_start_in_a_closed_project_refuses_before_the_card(cx: &mut gpui::TestAppContext) {
    let _fixture = workspace_with_agent_chat(cx);
    let task = add_task(ProjectUuid::new(), "closed", cx);
    cx.update(|cx| {
        pair_bridge(cx);
        let outcome = ready(run_gated(GatedCommand::TaskStart { task }, cx));
        assert_eq!(outcome, Err(ControlError::TaskProjectNotOpen));
        assert_eq!(crate::control::approval::waiting_count_for_test(cx), 0);
    });
}

/// A refused start leaves the task where it was.
#[gpui::test]
async fn a_refused_start_runs_nothing(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let (uuid, _) = project_of(&fixture, cx);
    let task = add_task(uuid, "t", cx);
    let dispatch = cx.update(|cx| {
        pair_bridge(cx);
        run_gated(GatedCommand::TaskStart { task: task.clone() }, cx)
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
            cx.global::<GlobalTasks>().get(&task).unwrap().state,
            TaskState::Backlog
        );
    });
}

#[gpui::test]
async fn only_a_running_task_stops(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let (uuid, _) = project_of(&fixture, cx);
    let task = add_task(uuid, "t", cx);
    let outcome = cx.update(|cx| run(ResolvedCommand::TaskStop { task }, cx));
    assert_eq!(outcome, Err(ControlError::TaskNotRunning));
    let outcome = cx.update(|cx| {
        run(
            ResolvedCommand::TaskStop {
                task: "missing".into(),
            },
            cx,
        )
    });
    assert_eq!(outcome, Err(ControlError::TaskNotFound));
}

/// An approved start answers once the task runs: the worktree it runs in,
/// and the chat it runs as.
#[gpui::test]
async fn an_approved_start_answers_with_its_worktree_and_chat(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let (workspace, lane) = fixture
        .workspace
        .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane()));
    let task = fixture.workspace.update(cx, |ws, cx| {
        ws.control_task_create(lane.project, "Here", "p".into(), Some(lane), cx)
            .expect("created")
    });
    let dispatch = cx.update(|cx| {
        pair_bridge(cx);
        run_gated(GatedCommand::TaskStart { task: task.clone() }, cx)
    });
    let Dispatch::Deferred { outcome: rx, .. } = dispatch else {
        panic!("a start asks first");
    };
    cx.update(|cx| resolve_only_pending_for_test(ApprovalChoice::Approved, cx));
    cx.run_until_parked();
    let Ok(Some(Ok(ControlResult::TaskStarted {
        task: started,
        lane: handle,
        chat: Some(chat),
    }))) = rx.recv().await
    else {
        panic!("not started");
    };
    assert_eq!(started, task);
    assert_eq!(handle, LaneHandle::new(workspace, lane));
    assert_eq!(chat.workspace, workspace);
    // It ran: the test agent's session then fails, which is the task's own
    // later lifecycle, not this answer's.
    cx.update(|cx| {
        let task = cx.global::<GlobalTasks>().get(&task).unwrap();
        assert_ne!(task.state, TaskState::Backlog);
        assert!(task.execution.is_some(), "the run is bound to the task");
    });
}
