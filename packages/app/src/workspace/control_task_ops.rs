//! Tasks on behalf of the control surface.
//!
//! The start and open paths are the Tasks UI's own (`begin_task_start`,
//! `begin_task_chat_open`); this file only names what they answer in the
//! control vocabulary. That translation lives inside `crate::workspace`
//! because the answers' types do.

use gpui::{Context, Window};

use daruda_store::project::{LaneRef, ProjectId, ProjectUuid};
use daruda_store::tasks::{ExecutionRef, Lifecycle, StartedBy, TaskSpec};

use crate::control::result::{ControlError, TaskProject};
use crate::control::spec::NewTask;
use crate::runs::{GlobalTasks, TaskCommitError, TaskIntent};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::right_dock::task_chat_ops::TaskChatError;
use crate::workspace::right_dock::task_start::{TaskStartError, TaskStarted};
use crate::workspace::task_draft::{DraftError, DraftLocation, TaskDraft};

/// A started execution, its worktree, and its chat when it has one.
pub(crate) type ControlTaskStart = Result<(ExecutionRef, LaneRef, Option<PaneId>), ControlError>;

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

    /// The spec `daruda_task_create` would add, checked as the Task editor
    /// checks its own: the same branch rules and the same project's lanes.
    /// Pure, so the approval guard can ask before the card is shown.
    pub(crate) fn control_task_spec(
        &self,
        project: ProjectId,
        request: NewTask,
    ) -> Result<TaskSpec, ControlError> {
        let uuid = self
            .project_for(project)
            .map(|p| p.uuid)
            .ok_or(ControlError::TargetGone)?;
        // A worktree of another project would leave the task pointing at a
        // repository it does not belong to.
        let location = match request.worktree.map(|lane| lane.lane_ref()) {
            None => DraftLocation::NewWorktree { base: None },
            Some(lane) if lane.project == project => DraftLocation::ExistingLane(
                self.lane_for(lane)
                    .map(|l| l.path.clone())
                    .ok_or(ControlError::TargetGone)?,
            ),
            Some(_) => return Err(ControlError::TargetGone),
        };
        let draft = TaskDraft {
            title: request.title,
            prompt: request.prompt,
            notes: request.notes.unwrap_or_default(),
            branch: request.branch.unwrap_or_default(),
            location,
            agent_surface: request.surface.unwrap_or_default(),
            auto_execute: true,
        };
        self.task_spec_from(uuid, draft).map_err(|e| match e {
            DraftError::TitleEmpty => ControlError::TaskTitleEmpty,
            DraftError::ProjectNotOpen | DraftError::LaneNotInProject => ControlError::TargetGone,
            DraftError::BranchInvalid => ControlError::LaneNameInvalid,
            DraftError::BranchTaken => ControlError::TaskBranchTaken,
        })
    }

    /// Add a Backlog task to `project`, saved like one the form creates.
    /// Answers once the task is in the log, so the caller's next command
    /// finds it.
    pub(crate) fn control_task_create(
        &mut self,
        project: ProjectId,
        request: NewTask,
        cx: &mut Context<Self>,
    ) -> Result<gpui::Task<Result<String, ControlError>>, ControlError> {
        let spec = self.control_task_spec(project, request)?;
        let id = spec.id.clone();
        let created = self.propose_task_command(TaskIntent::Create { spec }, cx);
        Ok(cx.spawn(async move |_, _| created.await.map(|_| id).map_err(control_commit_error)))
    }

    /// Cancel a task that is starting or running, as the Tasks UI's Cancel
    /// does. Answers once the cancel is in the log.
    pub(crate) fn control_task_stop(
        &mut self,
        task: &str,
        cx: &mut Context<Self>,
    ) -> Result<gpui::Task<Result<(), ControlError>>, ControlError> {
        let exec = match cx.global::<GlobalTasks>().get(task).map(|t| &t.lifecycle) {
            None => return Err(ControlError::TaskNotFound),
            Some(
                Lifecycle::Preparing { exec, .. }
                | Lifecycle::Dispatching { exec, .. }
                | Lifecycle::Running { exec, .. },
            ) => exec.clone(),
            Some(_) => return Err(ControlError::TaskNotRunning),
        };
        let cancelled = self.cancel_execution(exec, cx);
        Ok(cx.spawn(async move |_, _| {
            cancelled.await.map(|_| ()).map_err(|e| match e {
                TaskCommitError::Rejected(_) => ControlError::TaskNotRunning,
                other => control_commit_error(other),
            })
        }))
    }

    /// Start `task`, answering on the returned channel. A refusal is on it
    /// before this returns, as with [`Self::begin_task_start`].
    pub(crate) fn control_task_start(
        &mut self,
        task: &str,
        started_by: StartedBy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> smol::channel::Receiver<ControlTaskStart> {
        let (tx, rx) = smol::channel::bounded(1);
        let started = self.begin_task_start(task, started_by, window, cx);
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
        Ok((started.exec, lane, chat))
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

/// A task change the log did not take, in the control vocabulary.
fn control_commit_error(e: TaskCommitError) -> ControlError {
    ControlError::TaskNotRecorded {
        detail: e.to_string(),
    }
}

/// A start failure in the control vocabulary. The detail strings are for
/// the model reading the result, not for a person, so they stay English.
fn control_start_error(e: TaskStartError) -> ControlError {
    match e {
        TaskStartError::RemoteLane => ControlError::TaskStartFailed {
            detail: daruda_store::tasks::TaskRejected::RemoteLane.to_string(),
        },
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
        TaskStartError::NotRecorded { detail } => ControlError::TaskNotRecorded { detail },
    }
}
