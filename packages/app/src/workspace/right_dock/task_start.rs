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
use daruda_store::tasks::TaskAgentSurface;
use gpui::{BorrowAppContext, Context, Window};

use crate::agent::tasks_global::GlobalTasks;
use crate::workspace::Workspace;
use crate::workspace::lane_ops::CreateWorktreePlan;
use crate::workspace::main_area::agent_chat_pane::view::AgentSessionStatus;
use crate::workspace::main_area::pane_tree::PaneId;

/// Why a task did not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TaskStartError {
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
    /// `Error` with `detail` as its message.
    PromptUndelivered {
        detail: String,
    },
    /// The window went away while the worktree was being created.
    WindowClosed,
}

/// A task that is now running, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskStarted {
    pub(crate) worktree: PathBuf,
    pub(crate) pane: PaneId,
    pub(crate) surface: TaskAgentSurface,
}

pub(crate) type TaskStartOutcome = Result<TaskStarted, TaskStartError>;

type Answer = smol::channel::Sender<TaskStartOutcome>;

impl TaskStartError {
    /// The desktop toast for this failure; `None` where the UI already shows
    /// it — a row in `Error`, a spawn failure its own path reported — or
    /// where the Tasks UI cannot ask for it (an unknown or started task).
    fn report(&self) -> Option<ErrorReport> {
        use crate::surface::strings::error;
        let (message, severity, dedup) = match self {
            Self::NotFound
            | Self::NotBacklog
            | Self::PaneUnavailable
            | Self::PromptUndelivered { .. }
            | Self::WindowClosed => return None,
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

    /// Start a Backlog task: open the lane (`git worktree add`, or a new
    /// tab in an existing lane for `TaskRunIn::ExistingLane`), then write
    /// the prompt file and dispatch the `claude` command into the pane.
    ///
    /// Answers on the returned channel once the pane holds the prompt. A
    /// refusal is on it before this returns. A caller that stops listening
    /// cannot stall it: the channel holds one answer and is never awaited.
    pub(crate) fn begin_task_start(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> smol::channel::Receiver<TaskStartOutcome> {
        let (answer, rx) = smol::channel::bounded(1);
        if let Err(e) = self.start_task_into(task_id, answer.clone(), window, cx) {
            self.finish_task_start(Err(e), &answer, cx);
        }
        rx
    }

    /// The one place a start's outcome is delivered: the toast, then the
    /// answer.
    fn finish_task_start(
        &mut self,
        outcome: TaskStartOutcome,
        answer: &Answer,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = &outcome
            && let Some(report) = e.report()
        {
            self.report_error(report, cx);
        }
        // SILENT-OK: a full or closed channel means the caller stopped
        // listening; the start itself is done either way.
        let _ = answer.try_send(outcome);
    }

    /// `Err` is a refusal taken before anything was spawned; a start that
    /// got as far as git answers on `answer` itself.
    fn start_task_into(
        &mut self,
        task_id: &str,
        answer: Answer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), TaskStartError> {
        // Snapshot every read off `self` we'll need inside the spawn —
        // the workspace isn't borrowable across `await` points.
        let task = cx
            .global::<GlobalTasks>()
            .get(task_id)
            .cloned()
            .ok_or(TaskStartError::NotFound)?;
        if !matches!(task.state, daruda_store::tasks::TaskState::Backlog) {
            return Err(TaskStartError::NotBacklog);
        }
        // An existing lane needs no git repo, lock or checkout.
        if let daruda_store::tasks::TaskRunIn::ExistingLane { path } = &task.run_in {
            let started = self.start_task_in_existing_lane(&task, path, window, cx)?;
            self.finish_task_start(Ok(started), &answer, cx);
            return Ok(());
        }
        let project_id = self
            .project_by_uuid(task.project)
            .map(|p| p.id)
            .ok_or(TaskStartError::ProjectNotOpen)?;
        let repo_root = self
            .git_repo_root_for_project(project_id)
            .ok_or(TaskStartError::NoGitRepo)?;
        if !self.acquire_repo_lock(&repo_root) {
            return Err(TaskStartError::RepoBusy);
        }

        let new_path =
            crate::workspace::lane_ops::lane_checkout_path(&repo_root, &task.branch_name);
        let base_ref = task
            .base_worktree_path
            .as_deref()
            .and_then(|p| self.branch_for_worktree_path(p));
        let plan = CreateWorktreePlan {
            branch: task.branch_name.clone(),
            new_path,
            repo_root,
            base_ref: self.resolve_lane_base_ref_for(project_id, base_ref),
            description: Some(crate::surface::strings::task::lane_description(&task.title)),
            // Task-driven lanes have no create-form host picker — they stay
            // at `Lane::git`'s default (unanswered/Local).
            session_host: None,
        };

        let task_id = task.id.clone();
        let agent_surface = task.agent_surface;
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
                        let outcome = result
                            .map_err(|detail| TaskStartError::GitAddFailed { detail })
                            .and_then(|()| {
                                ws.finalize_create_lane(
                                    plan.clone(),
                                    project_id,
                                    agent_surface,
                                    None,
                                    window,
                                    cx,
                                )
                                .map_err(|detail| TaskStartError::FinalizeFailed { detail })
                            })
                            .and_then(|pane| {
                                ws.dispatch_claude_for_task(
                                    &task_id,
                                    &plan.new_path,
                                    &plan.branch,
                                    pane,
                                    window,
                                    cx,
                                )?;
                                Ok(TaskStarted {
                                    worktree: plan.new_path.clone(),
                                    pane,
                                    surface: agent_surface,
                                })
                            });
                        ws.finish_task_start(outcome, &answer, cx);
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
                    // SILENT-OK: the caller may have stopped listening.
                    let _ = answer.try_send(Err(TaskStartError::WindowClosed));
                }
            })
            .detach();
        Ok(())
    }

    /// Write `<lane>/.daruda/task-<id>.md` and dispatch the
    /// `claude --dangerously-skip-permissions "$(cat '...')"` command
    /// into the freshly-created pane. Marks the task `Running` on
    /// success, `Error` when the prompt cannot be written or delivered.
    ///
    /// Takes `pane_id` rather than relying on `focused_pane_id` —
    /// `activate_lane` does call `focus_pane` on the happy path,
    /// but routing the command through the pane the lane spawned
    /// is bug-resistant against any future change to focus handling.
    pub(super) fn dispatch_claude_for_task(
        &mut self,
        task_id: &str,
        worktree_path: &Path,
        branch: &str,
        pane_id: crate::workspace::main_area::pane_tree::PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), TaskStartError> {
        let Some(task) = cx.global::<GlobalTasks>().get(task_id).cloned() else {
            return Err(TaskStartError::NotFound);
        };
        let rendered = daruda_store::tasks::prompt_file::render_task_prompt(&task, branch);
        // Whether the prompt reached the pane. Each surface reports it; a
        // `false` (pane closed / kind can't receive) routes the task to `Error`
        // instead of a `Running` state that would never be driven. This is
        // currently unreachable — `dispatch_claude_for_task` runs synchronously
        // right after `finalize_create_lane` spawned the pane, with no `await`
        // in between — but capturing it makes a future async gap fail loudly
        // rather than silently stranding the task in `Running`.
        let delivered = match task.agent_surface {
            daruda_store::tasks::TaskAgentSurface::Terminal => {
                let prompt_path =
                    daruda_store::tasks::prompt_file::prompt_file_path(&task, worktree_path);
                if let Err(e) =
                    daruda_store::tasks::prompt_file::write_prompt_file(&prompt_path, &rendered)
                {
                    return Err(self.fail_task_dispatch(
                        task_id,
                        worktree_path,
                        crate::surface::strings::task::error_write_prompt(e.to_string()),
                        cx,
                    ));
                }
                self.bind_task_cli_execution(task_id, pane_id, worktree_path, cx);
                let cmd = daruda_store::tasks::prompt_file::build_claude_command(
                    &prompt_path,
                    task.auto_execute,
                );
                self.send_to_pane(pane_id, cmd.as_bytes())
            }
            daruda_store::tasks::TaskAgentSurface::AgentChat => {
                self.bind_task_chat_execution(task_id, pane_id, cx);
                let pane_in_error = self.agent_chat_view(pane_id).is_some_and(|view| {
                    matches!(view.read(cx).status(), AgentSessionStatus::Error { .. })
                });
                if pane_in_error {
                    return Err(self.fail_task_dispatch(
                        task_id,
                        worktree_path,
                        crate::surface::strings::task::error_prompt_undelivered(),
                        cx,
                    ));
                }
                // The ACP session *is* the agent — there is no `claude …` CLI
                // wrapper to build. Deliver the rendered prompt (the same text
                // the Terminal path writes into `task-<id>.md`) as an ACP
                // turn. `finalize_create_lane` already spawned + focused the
                // agent-chat pane, which started the lazy connect; the pane's
                // pending-prompt queue buffers this turn and drains it one-per-
                // turn once the session connects.
                //
                // `auto_execute` has no analogue here: a `submit: true` turn
                // runs automatically, and ACP has no dangerous-skip flag, so
                // the AgentChat surface is inherently "auto-execute".
                self.deliver_text_to_pane(
                    pane_id,
                    crate::workspace::main_area::pane_input_ops::PaneTextInput {
                        body: rendered,
                        intent:
                            crate::workspace::main_area::pane_input_ops::PaneTextIntent::Command {
                                submit: true,
                            },
                    },
                    window,
                    cx,
                )
            }
        };

        if !delivered {
            return Err(self.fail_task_dispatch(
                task_id,
                worktree_path,
                crate::surface::strings::task::error_prompt_undelivered(),
                cx,
            ));
        }

        cx.update_global::<GlobalTasks, _>(|g, _| {
            if let Some(t) = g.get_mut(task_id) {
                t.state = daruda_store::tasks::TaskState::Running {
                    worktree_path: worktree_path.to_path_buf(),
                };
                t.updated_at = Utc::now();
            }
        });
        self.save_tasks_dirty(cx);

        // Dynamic install: for the Terminal surface, if a TaskEdit pane for
        // this task is already open (the user clicked Start from the pane
        // footer), the prompt file just landed on disk for the first time —
        // install the FS watcher now instead of waiting for the user to
        // close-and-reopen the pane. The AgentChat surface writes no prompt
        // file, so this is a harmless no-op there (no path to watch).
        self.attach_prompt_watcher_if_pane_open(task_id, window, cx);

        cx.notify();
        Ok(())
    }

    /// Shared failure exit for [`Self::dispatch_claude_for_task`]: mark `task_id`
    /// `Error { worktree_path, message }`, then persist + notify. Extracted so
    /// the prompt-file-write failure and the pane-delivery failure set the same
    /// fields through one path. The row shows the message, so it raises no
    /// toast; the caller hands it back as the start's outcome.
    fn fail_task_dispatch(
        &mut self,
        task_id: &str,
        worktree_path: &Path,
        message: String,
        cx: &mut Context<Self>,
    ) -> TaskStartError {
        cx.update_global::<GlobalTasks, _>(|g, _| {
            if let Some(t) = g.get_mut(task_id) {
                t.state = daruda_store::tasks::TaskState::Error {
                    worktree_path: worktree_path.to_path_buf(),
                    message: message.clone(),
                };
                t.updated_at = Utc::now();
            }
        });
        self.save_tasks_dirty(cx);
        cx.notify();
        TaskStartError::PromptUndelivered { detail: message }
    }
}

#[cfg(test)]
#[path = "task_start_tests.rs"]
mod tests;
