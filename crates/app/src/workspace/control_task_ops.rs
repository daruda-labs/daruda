//! Tasks on behalf of the control surface.
//!
//! The start and open paths are the Tasks UI's own (`begin_task_start`,
//! `begin_task_chat_open`); this file only names what they answer in the
//! control vocabulary. That translation lives inside `crate::workspace`
//! because the answers' types do.

use gpui::{BorrowAppContext as _, Context, Window};

use daruda_store::project::{LaneRef, ProjectId, ProjectUuid};
use daruda_store::tasks::{Task, TaskRunIn, TaskState};

use crate::agent::tasks_global::GlobalTasks;
use crate::control::result::{ControlError, TaskProject};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::right_dock::task_chat_ops::TaskChatError;
use crate::workspace::right_dock::task_start::{TaskStartError, TaskStarted};

/// A started task: the worktree it runs in, and its chat when it has one.
pub(crate) type ControlTaskStart = Result<(LaneRef, Option<PaneId>), ControlError>;

impl Workspace {
    /// Each open project's durable id, with the handle a caller names it by.
    pub(crate) fn control_task_projects(&self) -> Vec<(ProjectUuid, TaskProject)> {
        self.projects
            .iter()
            .map(|p| {
                (
                    p.uuid,
                    TaskProject {
                        workspace: self.uuid(),
                        project: p.id,
                        name: p.name.clone(),
                    },
                )
            })
            .collect()
    }

    pub(crate) fn control_has_project_uuid(&self, uuid: ProjectUuid) -> bool {
        self.project_by_uuid(uuid).is_some()
    }

    /// Add a Backlog task to `project`, running in `worktree` when one is
    /// named. Saved like one the form creates, with the form's defaults.
    pub(crate) fn control_task_create(
        &mut self,
        project: ProjectId,
        title: &str,
        prompt: String,
        worktree: Option<LaneRef>,
        cx: &mut Context<Self>,
    ) -> Result<String, ControlError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(ControlError::TaskTitleEmpty);
        }
        let uuid = self
            .project_for(project)
            .map(|p| p.uuid)
            .ok_or(ControlError::TargetGone)?;
        // A worktree of another project would leave the task pointing at a
        // repository it does not belong to.
        let run_in = match worktree {
            None => TaskRunIn::NewWorktree,
            Some(lane) if lane.project == project => TaskRunIn::ExistingLane {
                path: self
                    .lane_for(lane)
                    .map(|l| l.path.clone())
                    .ok_or(ControlError::TargetGone)?,
            },
            Some(_) => return Err(ControlError::TargetGone),
        };
        let mut task = Task::new(uuid, title.to_owned(), prompt, None);
        task.run_in = run_in;
        let id = task.id.clone();
        cx.update_global::<GlobalTasks, _>(|g, _| {
            g.add(task);
        });
        self.save_tasks_dirty(cx);
        cx.notify();
        Ok(id)
    }

    /// Cancel a running task, as the Tasks UI's Cancel does.
    pub(crate) fn control_task_stop(
        &mut self,
        task: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), ControlError> {
        match cx.global::<GlobalTasks>().get(task).map(|t| &t.state) {
            None => Err(ControlError::TaskNotFound),
            Some(TaskState::Running { .. }) => {
                self.cancel_task(task, cx);
                Ok(())
            }
            Some(_) => Err(ControlError::TaskNotRunning),
        }
    }

    /// Start `task`, answering on the returned channel. A refusal is on it
    /// before this returns, as with [`Self::begin_task_start`].
    pub(crate) fn control_task_start(
        &mut self,
        task: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> smol::channel::Receiver<ControlTaskStart> {
        let (tx, rx) = smol::channel::bounded(1);
        let started = self.begin_task_start(task, window, cx);
        // SILENT-OK on each send below: a caller that stopped listening
        // has nothing left to tell.
        if let Ok(outcome) = started.try_recv() {
            let _ = tx.try_send(self.control_task_started(outcome));
            return rx;
        }
        cx.spawn(async move |ws, cx| {
            let outcome = started
                .recv()
                .await
                .unwrap_or(Err(TaskStartError::WindowClosed));
            let answer = ws
                .update(cx, |ws, _| ws.control_task_started(outcome))
                .unwrap_or(Err(ControlError::TargetGone));
            let _ = tx.try_send(answer);
        })
        .detach();
        rx
    }

    fn control_task_started(
        &self,
        outcome: Result<TaskStarted, TaskStartError>,
    ) -> ControlTaskStart {
        let started = outcome.map_err(control_start_error)?;
        let lane = self
            .lane_ref_at(&started.worktree)
            .ok_or(ControlError::TargetGone)?;
        let chat = (started.surface == daruda_store::tasks::TaskAgentSurface::AgentChat)
            .then_some(started.pane);
        Ok((lane, chat))
    }

    /// Bring up the chat `task`'s run belongs to.
    pub(crate) fn control_task_open(
        &mut self,
        task: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<PaneId, ControlError> {
        self.begin_task_chat_open(task, window, cx)
            .map_err(|e| match e {
                TaskChatError::NotFound => ControlError::TaskNotFound,
                TaskChatError::NoSession => ControlError::TaskNoSession,
                TaskChatError::AgentUnavailable => ControlError::TaskAgentUnavailable,
                TaskChatError::WorktreeMissing => ControlError::TargetGone,
            })
    }
}

/// A start failure in the control vocabulary. The detail strings are for
/// the model reading the result, not for a person, so they stay English.
fn control_start_error(e: TaskStartError) -> ControlError {
    match e {
        TaskStartError::NotFound => ControlError::TaskNotFound,
        TaskStartError::NotBacklog => ControlError::TaskNotBacklog,
        TaskStartError::ProjectNotOpen => ControlError::TaskProjectNotOpen,
        TaskStartError::RepoBusy => ControlError::LaneCreateBusy,
        TaskStartError::LaneMissing { .. } | TaskStartError::WindowClosed => {
            ControlError::TargetGone
        }
        TaskStartError::NoGitRepo => ControlError::TaskStartFailed {
            detail: "the project is not a git repository".into(),
        },
        TaskStartError::PaneUnavailable => ControlError::TaskStartFailed {
            detail: "no pane could be opened in the worktree".into(),
        },
        TaskStartError::GitAddFailed { detail }
        | TaskStartError::FinalizeFailed { detail }
        | TaskStartError::PromptUndelivered { detail } => ControlError::TaskStartFailed { detail },
    }
}
