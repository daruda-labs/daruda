//! Starting a Backlog task, answered with its outcome.
//!
//! One start path for every caller. The Tasks UI drops the outcome — a
//! failure has already raised its toast here — while the control surface
//! waits on it, because a phone cannot see a desktop toast. A refusal is
//! answered before git is touched, so a caller hears it in the same turn.

use std::path::{Path, PathBuf};

use chrono::Utc;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::tasks::machine::StartFailure;
use daruda_store::tasks::{
    CliProcessState, ExecutionRef, Lifecycle, TaskAgentSurface, TaskEvent, TaskExecutionSource,
    TaskRecord, TaskRunIn,
};
use gpui::{App, Context, Window};

use crate::runs::{GlobalTasks, RunsHost, TaskCommitError};
use crate::workspace::Workspace;
use crate::workspace::lane_ops::CreateWorktreePlan;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_chat::view::AgentSessionStatus;

/// Why a task did not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TaskStartError {
    RemoteLane,
    NotFound,
    /// Only a Backlog task starts; any other has run already.
    NotBacklog,
    /// The task's project is not open in the window asked to start it.
    ProjectNotOpen,
    NoGitRepo,
    /// Another worktree is being created in the same repository.
    RepoBusy,
    /// The existing lane the task runs in is gone or cannot be read.
    LaneMissing {
        path: PathBuf,
    },
    /// The lane is there, but no pane could be opened in it. Its own spawn
    /// path reported why.
    PaneUnavailable,
    GitAddFailed {
        detail: String,
    },
    FinalizeFailed {
        detail: String,
    },
    /// The pane opened but the prompt never reached it. The task is now
    /// `Error`; `detail` says why, for the control surface.
    PromptUndelivered {
        detail: String,
    },
    /// The window went away while the worktree was being created.
    WindowClosed,
    /// The task log did not record the start; nothing was spawned.
    NotRecorded {
        detail: String,
    },
}

/// Where an accepted start handed the task's prompt to a pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskStarted {
    pub(crate) worktree: PathBuf,
    pub(crate) pane: PaneId,
    pub(crate) surface: TaskAgentSurface,
}

pub(crate) type TaskStartOutcome = Result<TaskStarted, TaskStartError>;

type Answer = smol::channel::Sender<TaskStartOutcome>;

impl TaskStartError {
    /// The cause the task log records for a start that failed after it was
    /// requested; `None` where no run was recorded to fail.
    fn failure(&self) -> Option<StartFailure> {
        Some(match self {
            Self::NotFound => StartFailure::NotFound,
            Self::NotBacklog | Self::NotRecorded { .. } | Self::RemoteLane => return None,
            Self::ProjectNotOpen => StartFailure::ProjectNotOpen,
            Self::NoGitRepo => StartFailure::NoGitRepo,
            Self::RepoBusy => StartFailure::RepoBusy,
            Self::LaneMissing { .. } => StartFailure::LaneMissing,
            Self::PaneUnavailable => StartFailure::PaneUnavailable,
            Self::GitAddFailed { .. } => StartFailure::GitAddFailed,
            Self::FinalizeFailed { .. } => StartFailure::FinalizeFailed,
            Self::PromptUndelivered { .. } => StartFailure::PromptUndelivered,
            Self::WindowClosed => StartFailure::WindowClosed,
        })
    }

    /// The desktop toast for this failure; `None` where the UI already shows
    /// it — a row in `Error`, a spawn failure its own path reported — or
    /// where the Tasks UI cannot ask for it (an unknown or started task).
    fn report(&self) -> Option<ErrorReport> {
        use crate::surface::strings::error;
        let (message, severity, dedup) = match self {
            Self::RemoteLane => (
                crate::surface::strings::task::error_remote_lane(),
                ErrorSeverity::Info,
                "tasks.remote_lane",
            ),
            Self::NotFound
            | Self::NotBacklog
            | Self::PaneUnavailable
            | Self::PromptUndelivered { .. }
            | Self::WindowClosed => return None,
            Self::NotRecorded { detail } => {
                return Some(
                    ErrorReport::new(error::tasks_save_failed())
                        .severity(ErrorSeverity::Error)
                        .message(detail.clone())
                        .at(file!(), line!())
                        .dedup("tasks.start.not_recorded")
                        .build(),
                );
            }
            Self::ProjectNotOpen => (
                error::task_project_not_open(),
                ErrorSeverity::Info,
                "tasks.project_not_open",
            ),
            Self::NoGitRepo => (
                error::tasks_require_git_repo(),
                ErrorSeverity::Info,
                "tasks.no_git_repo",
            ),
            Self::RepoBusy => (
                error::lane_create_busy(),
                ErrorSeverity::Info,
                "lane.create.busy",
            ),
            Self::LaneMissing { path } => {
                return Some(
                    ErrorReport::new(error::task_lane_missing())
                        .severity(ErrorSeverity::Error)
                        .at(file!(), line!())
                        .with_context(
                            "path",
                            daruda_store::observability::system_info::redact_home(path),
                        )
                        .dedup("task.start.lane_missing")
                        .build(),
                );
            }
            Self::GitAddFailed { detail } | Self::FinalizeFailed { detail } => {
                let message = if matches!(self, Self::GitAddFailed { .. }) {
                    error::lane_create_failed()
                } else {
                    error::lane_finalize_failed()
                };
                return Some(
                    ErrorReport::new(message)
                        .severity(ErrorSeverity::Error)
                        .at(file!(), line!())
                        .with_context("detail", detail.clone())
                        .dedup("lane.create")
                        .build(),
                );
            }
        };
        Some(
            ErrorReport::new(message)
                .severity(severity)
                .at(file!(), line!())
                .dedup(dedup)
                .build(),
        )
    }
}

impl Workspace {
    /// Start a Backlog task from the Tasks UI. The outcome is dropped: every
    /// failure the user can act on has raised its toast by then.
    pub(in crate::workspace) fn start_task(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        drop(self.begin_task_start(task_id, window, cx));
    }

    /// Start a Backlog task: record the start, open the lane (`git worktree
    /// add`, or a new tab in an existing lane for `TaskRunIn::ExistingLane`),
    /// then hand the pane the prompt.
    ///
    /// Answers on the returned channel once the pane holds the prompt. A
    /// refusal is on it before this returns — unless task commits are still
    /// landing, which the start first waits for, so a task saved a moment ago
    /// is found. A caller that stops listening cannot stall it: the channel
    /// holds one answer and is never awaited.
    pub(crate) fn begin_task_start(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> smol::channel::Receiver<TaskStartOutcome> {
        let (answer, rx) = smol::channel::bounded(1);
        let started = cx
            .global::<GlobalTasks>()
            .get(task_id)
            .is_some_and(|task| task.lifecycle != Lifecycle::Backlog);
        if started {
            self.finish_task_start(Err(TaskStartError::NotBacklog), None, &answer, cx);
            return rx;
        }
        if RunsHost::settled(cx) {
            if let Err(e) = self.start_task_into(task_id, answer.clone(), window, cx) {
                self.finish_task_start(Err(e), None, &answer, cx);
            }
            return rx;
        }
        let barrier = RunsHost::barrier(cx);
        let id = task_id.to_owned();
        cx.spawn_in(window, async move |ws, cx| {
            barrier.await;
            let entered = ws.update_in(cx, |ws, window, cx| {
                if let Err(e) = ws.start_task_into(&id, answer.clone(), window, cx) {
                    ws.finish_task_start(Err(e), None, &answer, cx);
                }
            });
            if entered.is_err() {
                // SILENT-OK: the caller may have stopped listening.
                let _ = answer.try_send(Err(TaskStartError::WindowClosed));
            }
        })
        .detach();
        rx
    }

    /// The one place a start's outcome is delivered: the failure recorded
    /// for a run that was requested, the toast, then the answer.
    fn finish_task_start(
        &mut self,
        outcome: TaskStartOutcome,
        exec: Option<&ExecutionRef>,
        answer: &Answer,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = &outcome {
            if let Some(exec) = exec {
                fail_start(exec, e, cx);
            }
            if let Some(report) = e.report() {
                self.report_error(report, cx);
            }
        }
        // SILENT-OK: a full or closed channel means the caller stopped
        // listening; the start itself is done either way.
        let _ = answer.try_send(outcome);
    }

    /// Check the task can start, record the request, then continue once the
    /// log holds it. `Err` is a refusal taken before anything was recorded or
    /// spawned, leaving the task in Backlog.
    fn start_task_into(
        &mut self,
        task_id: &str,
        answer: Answer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), TaskStartError> {
        let task = cx
            .global::<GlobalTasks>()
            .get(task_id)
            .cloned()
            .ok_or(TaskStartError::NotFound)?;
        if task.lifecycle != Lifecycle::Backlog {
            return Err(TaskStartError::NotBacklog);
        }
        // An existing lane needs no git repo, lock or checkout.
        let route = match &task.spec.run_in {
            TaskRunIn::ExistingLane { path } => {
                StartRoute::Existing(self.prepare_existing_lane_start(path, window, cx)?)
            }
            TaskRunIn::NewWorktree => StartRoute::New(self.prepare_worktree_start(&task)?),
        };
        let exec = ExecutionRef {
            task_id: task.id().to_owned(),
            execution_id: uuid::Uuid::new_v4().to_string(),
        };
        let requested = RunsHost::task(
            crate::runs::TaskIntent::Start {
                exec: exec.clone(),
                source: source_for(task.spec.agent_surface),
            },
            cx,
        );
        cx.spawn_in(window, async move |ws, cx| {
            let recorded = requested.await;
            let continued = ws.update_in(cx, |ws, window, cx| match recorded {
                Ok(_) => ws.continue_task_start(route, exec.clone(), answer.clone(), window, cx),
                Err(e) => {
                    if let StartRoute::New(plan) = &route {
                        ws.release_repo_lock(&plan.plan.repo_root);
                    }
                    let refused = match e {
                        TaskCommitError::Rejected(_) => TaskStartError::NotBacklog,
                        other => TaskStartError::NotRecorded {
                            detail: other.to_string(),
                        },
                    };
                    ws.finish_task_start(Err(refused), None, &answer, cx);
                }
            });
            if continued.is_err() {
                abandon_start(&exec, &answer, cx);
            }
        })
        .detach();
        Ok(())
    }

    /// Everything a new worktree needs, checked before the start is
    /// recorded; holds the repository's lock from here on.
    fn prepare_worktree_start(
        &mut self,
        task: &TaskRecord,
    ) -> Result<WorktreeStart, TaskStartError> {
        let project_id = self
            .project_by_uuid(task.spec.project)
            .map(|p| p.id)
            .ok_or(TaskStartError::ProjectNotOpen)?;
        let repo_root = self
            .git_repo_root_for_project(project_id)
            .ok_or(TaskStartError::NoGitRepo)?;
        if !self.acquire_repo_lock(&repo_root) {
            return Err(TaskStartError::RepoBusy);
        }
        let spec = &task.spec;
        let new_path =
            crate::workspace::lane_ops::lane_checkout_path(&repo_root, &spec.branch_name);
        let base_ref = spec
            .base_worktree_path
            .as_deref()
            .and_then(|p| self.branch_for_worktree_path(p));
        let plan = CreateWorktreePlan {
            branch: spec.branch_name.clone(),
            new_path,
            repo_root,
            base_ref: self.resolve_lane_base_ref_for(project_id, base_ref),
            description: Some(crate::surface::strings::task::lane_description(&spec.title)),
            // Task-driven lanes have no create-form host picker — they stay
            // at `Lane::git`'s default (unanswered/Local).
            session_host: None,
        };
        Ok(WorktreeStart {
            plan,
            project_id,
            surface: spec.agent_surface,
        })
    }

    /// The start is recorded: create the worktree or open the pane, then
    /// dispatch. Every failure from here is recorded against `exec`.
    fn continue_task_start(
        &mut self,
        route: StartRoute,
        exec: ExecutionRef,
        answer: Answer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match route {
            StartRoute::Existing(lane) => {
                let started = self.start_task_in_existing_lane(&exec, lane, window, cx);
                self.answer_when_dispatched(started, exec, answer, cx);
            }
            StartRoute::New(start) => self.create_task_worktree(start, exec, answer, window, cx),
        }
    }

    fn create_task_worktree(
        &mut self,
        start: WorktreeStart,
        exec: ExecutionRef,
        answer: Answer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let WorktreeStart {
            plan,
            project_id,
            surface,
        } = start;
        let me = cx.weak_entity();
        window
            .spawn(cx, async move |async_cx| {
                let result: Result<(), String> = async_cx
                    .background_executor()
                    .spawn({
                        let plan = plan.clone();
                        async move {
                            crate::lane::git::add_lane(
                                &plan.repo_root,
                                &plan.new_path,
                                Some(&plan.branch),
                                plan.base_ref.as_deref(),
                            )
                            .map_err(|e| e.to_string())
                        }
                    })
                    .await;

                let update_result = async_cx.update(|window, app_cx| {
                    let Some(workspace) = me.upgrade() else {
                        return false;
                    };
                    workspace.update(app_cx, |ws, cx| {
                        // Lock is released *before* finalize_create_lane
                        // runs. If finalize fails the new git worktree is
                        // already on disk — daruda just doesn't know about
                        // it (`git worktree prune` cleans it up). Released
                        // so a retry against the same repo isn't blocked.
                        ws.release_repo_lock(&plan.repo_root);
                        let started = result
                            .map_err(|detail| TaskStartError::GitAddFailed { detail })
                            .and_then(|()| {
                                ws.finalize_create_lane(
                                    plan.clone(),
                                    project_id,
                                    surface,
                                    None,
                                    window,
                                    cx,
                                )
                                .map_err(|detail| TaskStartError::FinalizeFailed { detail })
                            })
                            .and_then(|pane| {
                                let dispatched = ws.dispatch_claude_for_task(
                                    &exec,
                                    &plan.new_path,
                                    &plan.branch,
                                    pane,
                                    window,
                                    cx,
                                )?;
                                Ok((
                                    TaskStarted {
                                        worktree: plan.new_path.clone(),
                                        pane,
                                        surface,
                                    },
                                    dispatched,
                                ))
                            });
                        ws.answer_when_dispatched(started, exec.clone(), answer.clone(), cx);
                    });
                    true
                });
                if !matches!(update_result, Ok(true)) {
                    LogWriter::log(
                        ErrorReport::new("Task workflow completion could not reach workspace")
                            .severity(ErrorSeverity::Warning)
                            .at(file!(), line!())
                            .with_context("error", format!("{update_result:?}"))
                            .dedup("task_workflow.completion.update_failed")
                            .build(),
                    );
                    abandon_start(&exec, &answer, async_cx);
                }
            })
            .detach();
    }

    /// Answer after the preparation and delivery result is projected. Chat
    /// queueing leaves the task Dispatching until its PromptSent arrives.
    fn answer_when_dispatched(
        &mut self,
        started: Result<(TaskStarted, gpui::Task<Result<u64, TaskStartError>>), TaskStartError>,
        exec: ExecutionRef,
        answer: Answer,
        cx: &mut Context<Self>,
    ) {
        let (started, dispatched) = match started {
            Ok(started) => started,
            Err(e) => {
                self.finish_task_start(Err(e), Some(&exec), &answer, cx);
                return;
            }
        };
        cx.spawn(async move |ws, cx| {
            let outcome = match dispatched.await {
                Ok(_) => Ok(started),
                Err(e) => Err(e),
            };
            let finished = ws.update(cx, |ws, cx| {
                ws.finish_task_start(outcome.clone(), None, &answer, cx)
            });
            if finished.is_err() {
                // SILENT-OK: the caller may have stopped listening.
                let _ = answer.try_send(outcome);
            }
        })
        .detach();
    }

    /// Bind `pane_id` to the run, record the pane's identity, and hand it the
    /// prompt: a `claude` command over a prompt file for a terminal, an ACP
    /// turn for an Agent chat. Preparation is durable before delivery; only
    /// a successful PTY write or tagged PromptSent records Dispatched.
    ///
    /// Takes `pane_id` rather than relying on `focused_pane_id` —
    /// `activate_lane` does call `focus_pane` on the happy path,
    /// but routing the command through the pane the lane spawned
    /// is bug-resistant against any future change to focus handling.
    pub(super) fn dispatch_claude_for_task(
        &mut self,
        exec: &ExecutionRef,
        worktree_path: &Path,
        branch: &str,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<gpui::Task<Result<u64, TaskStartError>>, TaskStartError> {
        // A cancel that landed while the lane was made ends the start here.
        let task = cx
            .global::<GlobalTasks>()
            .get(&exec.task_id)
            .filter(|task| matches!(&task.lifecycle, Lifecycle::Preparing { exec: current, .. } if current == exec))
            .cloned()
            .ok_or(TaskStartError::NotBacklog)?;
        let surface = match &task.lifecycle {
            Lifecycle::Preparing {
                source: TaskExecutionSource::AgentChat,
                ..
            } => TaskAgentSurface::AgentChat,
            _ => TaskAgentSurface::Terminal,
        };
        let identity = match surface {
            TaskAgentSurface::Terminal => {
                self.bind_task_cli_execution(exec, pane_id, worktree_path, cx)
            }
            TaskAgentSurface::AgentChat => self.bind_task_chat_execution(exec, pane_id, cx),
        }
        .ok_or(TaskStartError::PaneUnavailable)?;
        let prepared = RunsHost::propose(
            TaskEvent::LanePrepared {
                exec: exec.clone(),
                worktree: worktree_path.to_path_buf(),
                agent_id: identity.agent_id,
                account_id: identity.account_id,
                cwd: identity.cwd,
                at: Utc::now(),
            },
            cx,
        );
        let exec = exec.clone();
        let worktree = worktree_path.to_path_buf();
        let branch = branch.to_owned();
        Ok(cx.spawn_in(window, async move |ws, cx| {
            let seq = prepared
                .await
                .map_err(|error| TaskStartError::NotRecorded {
                    detail: error.to_string(),
                })?;
            let result = match ws.update_in(cx, |ws, window, cx| {
                ws.deliver_prepared_task(
                    &exec,
                    &task,
                    &worktree,
                    &branch,
                    pane_id,
                    surface,
                    identity.account_id,
                    seq,
                    window,
                    cx,
                )
            }) {
                Ok(Ok(dispatched)) => dispatched.await,
                Ok(Err(error)) => Err(error),
                Err(_) => Err(TaskStartError::WindowClosed),
            };
            if let Err(error) = &result {
                gpui::AsyncApp::update(cx, |cx| fail_start(&exec, error, cx));
            }
            result
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn deliver_prepared_task(
        &mut self,
        exec: &ExecutionRef,
        task: &TaskRecord,
        worktree_path: &Path,
        branch: &str,
        pane_id: PaneId,
        surface: TaskAgentSurface,
        account_id: Option<daruda_store::accounts::AccountId>,
        prepared_seq: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<gpui::Task<Result<u64, TaskStartError>>, TaskStartError> {
        if RunsHost::pane_execution(self.run_pane(pane_id), cx) != Some(exec) {
            return Err(TaskStartError::PaneUnavailable);
        }
        if !cx.global::<GlobalTasks>().get(&exec.task_id).is_some_and(|task| {
        matches!(&task.lifecycle, Lifecycle::Dispatching { exec: current, .. } if current == exec)
    }) {
        return Err(TaskStartError::NotBacklog);
    }
        let rendered = daruda_store::tasks::prompt_file::render_task_prompt(task, branch);
        let dispatched = match surface {
            TaskAgentSurface::Terminal => {
                let prompt_path =
                    daruda_store::tasks::prompt_file::prompt_file_path(&task.spec, worktree_path);
                daruda_store::tasks::prompt_file::write_prompt_file(&prompt_path, &rendered)
                    .map_err(|error| TaskStartError::PromptUndelivered {
                        detail: crate::surface::strings::task::error_write_prompt(
                            error.to_string(),
                        ),
                    })?;
                self.track_task_cli_execution(pane_id, account_id, cx);
                let command = daruda_store::tasks::prompt_file::build_claude_command(
                    &prompt_path,
                    task.spec.auto_execute,
                );
                let Some(written) = self.write_task_input(pane_id, command.into_bytes()) else {
                    RunsHost::delivery_failed(
                        exec.clone(),
                        crate::runs::DeliveryFailure::Dropped(
                            daruda_store::tasks::machine::UndeliveredReason::PtyWriteFailed,
                        ),
                        cx,
                    );
                    return Err(TaskStartError::PromptUndelivered {
                        detail: crate::surface::strings::task::error_prompt_undelivered(),
                    });
                };
                let dispatched = RunsHost::terminal_dispatch(exec.clone(), written, cx);
                cx.spawn(async move |_, _| {
                    dispatched.await.map_err(|error| match error {
                        crate::runs::TerminalDispatchError::Commit(error) => {
                            TaskStartError::NotRecorded {
                                detail: error.to_string(),
                            }
                        }
                        crate::runs::TerminalDispatchError::Write(detail) => {
                            TaskStartError::PromptUndelivered { detail }
                        }
                    })
                })
            }
            TaskAgentSurface::AgentChat => {
                let failed = self.agent_chat_view(pane_id).is_some_and(|view| {
                    matches!(view.read(cx).status(), AgentSessionStatus::Error { .. })
                });
                if failed || !self.deliver_text_to_pane(pane_id, crate::workspace::main_area::pane_input_ops::PaneTextInput {
            body: rendered,
            intent: crate::workspace::main_area::pane_input_ops::PaneTextIntent::Command { submit: true },
            task: Some(exec.clone()),
        }, window, cx) {
            RunsHost::delivery_failed(exec.clone(), crate::runs::DeliveryFailure::Dropped(daruda_store::tasks::machine::UndeliveredReason::StartFailed), cx);
            return Err(TaskStartError::PromptUndelivered { detail: crate::surface::strings::task::error_prompt_undelivered() });
        }
                gpui::Task::ready(Ok(prepared_seq))
            }
        };

        // Dynamic install: for the Terminal surface, if the Task editor for
        // this task is open (the user clicked Start from its footer), the
        // prompt file just landed on disk for the first time —
        // install the FS watcher now instead of waiting for the user to
        // close-and-reopen the pane. The AgentChat surface writes no prompt
        // file, so this is a harmless no-op there (no path to watch).
        self.attach_prompt_watcher_if_editor_open(&exec.task_id, window, cx);

        cx.notify();
        Ok(dispatched)
    }
}

/// A new worktree's start, checked and holding the repository's lock.
struct WorktreeStart {
    plan: CreateWorktreePlan,
    project_id: daruda_store::project::ProjectId,
    surface: TaskAgentSurface,
}

/// Where a recorded start continues.
enum StartRoute {
    Existing(super::task_existing_lane_ops::ExistingLaneStart),
    New(WorktreeStart),
}

/// What the pane a run uses is, as `LanePrepared` records it.
pub(super) struct PaneIdentity {
    pub(super) agent_id: String,
    pub(super) account_id: Option<daruda_store::accounts::AccountId>,
    pub(super) cwd: PathBuf,
}

fn source_for(surface: TaskAgentSurface) -> TaskExecutionSource {
    match surface {
        TaskAgentSurface::AgentChat => TaskExecutionSource::AgentChat,
        TaskAgentSurface::Terminal => TaskExecutionSource::ClaudeCli {
            transcript_path: None,
            process: CliProcessState::Discovering,
        },
    }
}

/// Record why a requested start did not dispatch.
fn fail_start(exec: &ExecutionRef, e: &TaskStartError, cx: &mut App) {
    let Some(cause) = e.failure() else { return };
    RunsHost::delivery_failed(exec.clone(), crate::runs::DeliveryFailure::Start(cause), cx);
}

/// The window closed under a recorded start: record it, then answer.
fn abandon_start(exec: &ExecutionRef, answer: &Answer, cx: &gpui::AsyncApp) {
    cx.update(|cx| abandon_start_now(exec, answer, cx));
}

fn abandon_start_now(exec: &ExecutionRef, answer: &Answer, cx: &mut App) {
    fail_start(exec, &TaskStartError::WindowClosed, cx);
    // SILENT-OK: the caller may have stopped listening.
    let _ = answer.try_send(Err(TaskStartError::WindowClosed));
}

#[cfg(test)]
#[path = "task_start_tests.rs"]
mod tests;
