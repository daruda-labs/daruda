//! Right-panel Tasks tab — lifecycle layer.
//!
//! Hosts the task workflows after Start (which is `task_start`'s):
//! `cancel_task`, `focus_task_lane`, `reopen_task`, `retry_task`, plus
//! the supporting Claude-Code session bookkeeping
//! (`apply_task_session_changed`, `apply_task_session_ended`,
//! `apply_todo_write`, …).
//!
//! Sibling of `task_ops` (filter + CRUD). Both modules
//! extend the same `Workspace` via separate `impl Workspace` blocks.

use std::path::Path;
use std::time::Duration;

use crate::runs::{GlobalTasks, RunsHost};
use chrono::Utc;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::tasks::machine::{SettleOutcome, TodoItem as MergedTodo};
use daruda_store::tasks::{ExecutionRef, Lifecycle, TaskEvent, TaskRecord, new_task_id};
use gpui::{Context, Window};
use serde::Deserialize;

use crate::workspace::Workspace;

pub(in crate::workspace) const TASK_LIVE_TICK: Duration = Duration::from_millis(250);

/// Subset of the `TodoWrite` tool's `tool_input.todos[]` shape.
/// Claude Code includes more fields (`activeForm`, sometimes free-form
/// metadata) but daruda only cares about title + completion state.
/// `#[serde(default)]` on `status` keeps the parser robust against
/// minor schema drift — missing / unknown status reads as not-completed.
#[derive(Clone, Debug, Deserialize)]
struct TodoItem {
    #[serde(default)]
    content: String,
    #[serde(default)]
    status: String,
}

/// `tool_input` envelope from `PostToolUse { tool_name: "TodoWrite" }`.
#[derive(Clone, Debug, Deserialize)]
struct TodoWritePayload {
    #[serde(default)]
    todos: Vec<TodoItem>,
}

impl Workspace {
    // ------------------------------------------------------------------
    // Lifecycle (start / cancel / focus / reopen / retry)
    // ------------------------------------------------------------------

    /// Per-repo lock acquisition. Two concurrent `start_task` calls
    /// against the same repo race on `git worktree add`; we make the
    /// second one fail fast with a user-visible error rather than
    /// risk a half-created lane.
    ///
    /// `pub(super)` because the race is a property of the
    /// repository, not of this path: an agent-requested creation
    /// (`control_lane_ops`) has to take the same lock or it can interleave
    /// with the user's own.
    pub(in crate::workspace) fn acquire_repo_lock(&mut self, repo_root: &Path) -> bool {
        self.pending_lane_creates.insert(repo_root.to_path_buf())
    }

    pub(in crate::workspace) fn release_repo_lock(&mut self, repo_root: &Path) {
        self.pending_lane_creates.remove(repo_root);
    }

    /// Resolve the branch name we should base the new lane on.
    /// Looks the path up in the workspace's lane list and returns
    /// `lane.branch.clone()` when it's a git worktree. `None`
    /// is passed to `resolve_lane_base_ref`, which falls back to
    /// the project's `base_branch`, then `default_branch`, then
    /// the current HEAD.
    pub(super) fn branch_for_worktree_path(&self, path: &Path) -> Option<String> {
        self.projects
            .iter()
            .flat_map(|p| &p.lanes)
            .find(|w| w.path == path)
            .and_then(|w| match &w.kind {
                daruda_store::project::LaneKind::Git { branch, .. } => branch.clone(),
                _ => None,
            })
    }

    pub(super) fn write_task_input(
        &self,
        pane_id: crate::workspace::main_area::pane_tree::PaneId,
        bytes: Vec<u8>,
    ) -> Option<std::sync::mpsc::Receiver<Result<(), daruda_terminal::pty::PtyError>>> {
        self.active_runtime()
            .panes
            .iter()
            .find(|pane| pane.id == pane_id)?
            .write_confirmed_input(bytes)
    }

    /// User-initiated stop. Lane is preserved (D-1). The log records the
    /// cancel before the run's pane is told to stop.
    pub(in crate::workspace) fn cancel_task(&mut self, task_id: &str, cx: &mut Context<Self>) {
        let Some(task) = cx.global::<GlobalTasks>().get(task_id) else {
            return;
        };
        let cleared_sessions = task.session_ids.clone();
        let exec = task.current_exec().cloned();
        // Drop pending PostToolUseFailure tallies — ULIDs are never reused,
        // so leaving them would slowly accumulate dead entries across many
        // cancel/reopen cycles.
        self.claude_sessions
            .forget_tool_use_failures(&cleared_sessions);
        match exec {
            Some(exec) => self.cancel_execution(exec, cx).detach(),
            // Nothing runs, so the log refuses; the refusal is the report.
            None => {
                let refused = crate::runs::TaskCommitError::Rejected(
                    daruda_store::tasks::TaskRejected::IllegalTransition {
                        from: task_kind(cx, task_id),
                        event: daruda_store::tasks::machine::EventKind::CancelRequested,
                    },
                );
                self.report_error(super::task_commit_ops::task_commit_report(&refused), cx);
            }
        }
    }

    /// Switch the active lane to the one this task is running in. A running
    /// task whose checkout was deleted from the outside is interrupted
    /// (`LaneLost`) rather than dangling in `Running` forever; a finished
    /// task keeps its state.
    pub(in crate::workspace) fn focus_task_lane(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = cx.global::<GlobalTasks>().get(task_id) else {
            return;
        };
        let Some(path) = task.lifecycle.worktree().map(ToOwned::to_owned) else {
            return;
        };
        if !path.exists() {
            if let Lifecycle::Running { exec, .. } = &task.lifecycle {
                let cleared = task.session_ids.clone();
                let event = TaskEvent::LaneLost {
                    exec: exec.clone(),
                    at: Utc::now(),
                };
                self.claude_sessions.forget_tool_use_failures(&cleared);
                RunsHost::propose_fact(event, cx);
            }
            return;
        }

        // In any project: a task's lane need not be in the active one.
        if let Some(target) = self.lane_ref_at(&path) {
            self.activate_lane(target, window, cx);
        }
    }

    /// Move a terminal-state task back to `Backlog`; the next [Start] runs
    /// it again (see `return_task_to_backlog` for where).
    pub(in crate::workspace) fn reopen_task(&mut self, task_id: &str, cx: &mut Context<Self>) {
        self.return_task_to_backlog(task_id, cx).detach();
    }

    /// `Reopen` + `start_task` in one click — for the `[Retry]`
    /// affordance on failed rows. The start waits for the reopen to land.
    pub(super) fn retry_task(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reopened = self.return_task_to_backlog(task_id, cx);
        let id = task_id.to_owned();
        cx.spawn_in(window, async move |ws, cx| {
            if reopened.await.is_err() {
                return;
            }
            // SILENT-OK: the window closed; nothing is left to start in.
            let _ = ws.update_in(cx, |ws, window, cx| ws.start_task(&id, window, cx));
        })
        .detach();
    }

    /// Put a finished task back in Backlog so it can run again — in the lane
    /// its earlier run created, while that lane is still registered.
    fn return_task_to_backlog(
        &mut self,
        task_id: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<u64, crate::runs::TaskCommitError>> {
        let task = cx.global::<GlobalTasks>().get(task_id);
        let run_in = task.map(|task| match self.own_lane_for_rerun(task) {
            Some(path) => daruda_store::tasks::TaskRunIn::ExistingLane { path },
            None => task.spec.run_in.clone(),
        });
        let cleared = task
            .map(|task| task.session_ids.clone())
            .unwrap_or_default();
        self.claude_sessions.forget_tool_use_failures(&cleared);
        let intent = crate::runs::TaskIntent::Reopen {
            id: task_id.to_owned(),
            run_in: run_in.unwrap_or_default(),
        };
        self.propose_task_command(intent, cx)
    }

    // ------------------------------------------------------------------
    // Claude hook → task state mapping
    // ------------------------------------------------------------------

    /// Attach `session_id` to the running CLI task whose run owns it.
    /// Proposed only when it is not attached yet, so the same hook firing
    /// twice does not grow the session list.
    pub(in crate::workspace) fn apply_task_session_changed(
        &mut self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) {
        let attach: Vec<ExecutionRef> = cx
            .global::<GlobalTasks>()
            .records()
            .filter(|task| {
                matches!(task.lifecycle, Lifecycle::Running { .. })
                    && task
                        .execution
                        .as_ref()
                        .is_some_and(|run| run.source.cli_process().is_some())
                    && task.execution.as_ref().is_some_and(|run| {
                        run.source.cli_process().is_some()
                            && run.session_id.as_deref() == Some(session_id)
                    })
                    && !task.session_ids.iter().any(|s| s == session_id)
            })
            .filter_map(|task| task.current_exec().cloned())
            .collect();
        for exec in attach {
            let event = TaskEvent::SessionAttached {
                exec,
                session_id: session_id.to_owned(),
                at: Utc::now(),
            };
            RunsHost::propose_fact(event, cx);
        }
    }

    /// Settle every running CLI task that owns `session_id` as `Done` /
    /// `Error`, depending on `reason`. Settling clears its session list, so a
    /// future `Retry` starts clean.
    pub(in crate::workspace) fn apply_task_session_ended(
        &mut self,
        session_id: &str,
        reason: daruda_store::tasks::SessionEndReason,
        cx: &mut Context<Self>,
    ) {
        // Cleanup any pending failure counter for this session
        // unconditionally — once Claude reports the session ended,
        // the counter is meaningless even if the task is already
        // in a terminal state (e.g. user cancelled before
        // SessionEnd arrived).
        self.claude_sessions
            .tool_use_failure_counts
            .remove(session_id);
        let settle: Vec<ExecutionRef> = cx
            .global::<GlobalTasks>()
            .records()
            .filter(|task| {
                task.execution
                    .as_ref()
                    .is_some_and(|run| run.source.cli_process().is_some())
                    && task.session_ids.iter().any(|s| s == session_id)
            })
            .filter_map(running_exec)
            .collect();
        for exec in settle {
            let event = TaskEvent::Settled {
                exec,
                outcome: SettleOutcome::Completed { end_reason: reason },
                at: Utc::now(),
            };
            RunsHost::propose_fact(event, cx);
        }
    }

    /// Settle the task whose run `pane_id` owns, from that pane's activity
    /// edge. Only the pane that dispatched this execution may settle it;
    /// restoring history or a stale execution's late event owns no new work.
    pub(in crate::workspace) fn settle_agent_chat_task(
        &mut self,
        pane_id: crate::workspace::main_area::pane_tree::PaneId,
        outcome: SettleOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(exec) = self.task_chat_execution(pane_id, cx) else {
            return;
        };
        RunsHost::settle_execution(exec, outcome, cx);
    }

    pub(in crate::workspace) fn fail_agent_chat_start(
        &mut self,
        pane_id: crate::workspace::main_area::pane_tree::PaneId,
        cause: daruda_store::tasks::machine::StartFailure,
        cx: &mut Context<Self>,
    ) {
        if let Some(exec) = RunsHost::pane_execution(self.run_pane(pane_id), cx).cloned() {
            RunsHost::delivery_failed(exec, crate::runs::DeliveryFailure::Start(cause), cx);
            if let Some(view) = self.agent_chat_view(pane_id).cloned() {
                view.update(cx, |view, cx| {
                    view.discard_pending_prompts(
                        daruda_chat::view::PromptDropReason::ConnectFailed,
                        cx,
                    )
                });
            }
        }
    }

    pub(in crate::workspace) fn fail_agent_chat_task(
        &mut self,
        pane_id: crate::workspace::main_area::pane_tree::PaneId,
        cx: &mut Context<Self>,
    ) {
        if let Some(exec) = RunsHost::pane_execution(self.run_pane(pane_id), cx).cloned() {
            RunsHost::delivery_failed(
                exec,
                crate::runs::DeliveryFailure::Dropped(
                    daruda_store::tasks::machine::UndeliveredReason::StartFailed,
                ),
                cx,
            );
        }
    }

    /// Map a single Claude `last_event` string → a daruda
    /// `SessionEndReason`, returning `None` for events that should
    /// not transition a task. `last_event` is the
    /// `hook_event_name` field from the status file.
    pub(in crate::workspace) fn classify_hook_end_reason(
        last_event: &str,
    ) -> Option<daruda_store::tasks::SessionEndReason> {
        match last_event {
            "Stop" => Some(daruda_store::tasks::SessionEndReason::Stop),
            "SessionEnd" => Some(daruda_store::tasks::SessionEndReason::Other),
            _ => None,
        }
    }

    /// Increment the `PostToolUseFailure` counter for `session_id`.
    /// When the count crosses
    /// [`daruda_store::tasks::TASK_TOOL_USE_FAILURE_THRESHOLD`] the matching
    /// `Running` task is escalated to `Error` and the counter is
    /// dropped — subsequent failures from the same session start
    /// fresh (e.g. after a Retry).
    ///
    /// Sessions that no `Running` task owns are skipped entirely — the
    /// hook watcher fires for every Claude session on the host (including
    /// CLI invocations and other IDE integrations outside daruda's task
    /// system), and counting those would emit a noisy
    /// `tasks.escalation.orphan` warning every 5 failures.
    pub(in crate::workspace) fn bump_tool_use_failure(
        &mut self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) {
        if !Self::session_owned_by_running_task(session_id, cx) {
            return;
        }
        let count = self
            .claude_sessions
            .tool_use_failure_counts
            .entry(session_id.to_string())
            .and_modify(|n| *n = n.saturating_add(1))
            .or_insert(1);
        let count = *count;
        if count >= daruda_store::tasks::TASK_TOOL_USE_FAILURE_THRESHOLD {
            self.claude_sessions
                .tool_use_failure_counts
                .remove(session_id);
            let message = crate::surface::strings::task::error_tool_use_failure(count);
            self.escalate_task_session_to_error(session_id, message, cx);
        }
    }

    /// Does any `Running` task carry `session_id` in its `session_ids`?
    /// Used as a guard before incrementing the tool-use-failure counter
    /// so sessions outside daruda's task system don't trigger
    /// escalation.
    fn session_owned_by_running_task(session_id: &str, cx: &Context<Self>) -> bool {
        cx.global::<GlobalTasks>().records().any(|task| {
            matches!(task.lifecycle, Lifecycle::Running { .. })
                && task
                    .execution
                    .as_ref()
                    .is_some_and(|run| run.source.cli_process().is_some())
                && task.session_ids.iter().any(|s| s == session_id)
        })
    }

    /// Escalate every running task that owns `session_id` to `Error` for
    /// its tool failures. Distinct from a session that ended in error: the
    /// cause names why daruda ended the run itself.
    pub(super) fn escalate_task_session_to_error(
        &mut self,
        session_id: &str,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let owners: Vec<&TaskRecord> = cx
            .global::<GlobalTasks>()
            .records()
            .filter(|task| task.session_ids.iter().any(|s| s == session_id))
            .collect();
        let matched = !owners.is_empty();
        let escalate: Vec<ExecutionRef> = owners.into_iter().filter_map(running_exec).collect();
        if !matched {
            // Hook ordering can land PostToolUseFailure before the
            // first StatusChanged that attaches the session to a
            // task. The threshold has been hit but no Running task
            // owns this session — surface it so the silent drop is
            // observable. The session counter has already been
            // cleared by the caller, so we don't escalate again
            // when the next failure arrives.
            let report = ErrorReport::new(crate::surface::strings::error::task_escalation_orphan())
                .severity(ErrorSeverity::Warning)
                .message(
                    crate::surface::strings::error::task_escalation_orphan_detail(
                        session_id, &message,
                    ),
                )
                .at(file!(), line!())
                .with_context("session", session_id)
                .with_context("escalation", &message)
                .dedup("tasks.escalation.orphan")
                .build();
            self.report_error(report, cx);
        }
        for exec in escalate {
            let event = TaskEvent::ToolFailuresEscalated {
                exec,
                at: Utc::now(),
            };
            RunsHost::propose_fact(event, cx);
        }
    }

    // ------------------------------------------------------------------
    // TodoWrite hook auto-merge
    // ------------------------------------------------------------------

    /// Merge a `TodoWrite` hook payload into the matching task's
    /// subtasks.
    ///
    /// Routing: the workspace already tracks which Claude session id
    /// belongs to which `Running` task via `session_ids`. We
    /// reuse that linkage to find the target task — no extra
    /// bookkeeping needed.
    ///
    /// Policy (namespace isolation):
    /// - User-added subtasks (`source_session_id == None`) are
    ///   **never** touched, even if a hook todo carries the same title.
    /// - For matches against an existing auto-subtask we only flip
    ///   `completed`; titles aren't rewritten (Claude sometimes shifts
    ///   wording across emissions and we don't want the row text to
    ///   jitter under the user's cursor).
    /// - Unknown titles for this session_id are pushed as new auto
    ///   rows. Items Claude drops on subsequent emissions stay put —
    ///   no stale marking.
    ///
    /// Payload parse failures are silently dropped — Claude can change
    /// the `TodoWrite` schema at any time and a broken hook must not
    /// crash daruda. The parse failure is recorded as `Info` so the
    /// next bug report has a trail.
    pub(in crate::workspace) fn apply_todo_write(
        &mut self,
        session_id: &str,
        tool_input: &serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let payload: TodoWritePayload = match serde_json::from_value(tool_input.clone()) {
            Ok(p) => p,
            Err(e) => {
                LogWriter::log(
                    ErrorReport::new("TodoWrite payload parse failed")
                        .severity(ErrorSeverity::Info)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("session", session_id)
                        .dedup("hooks.todowrite.parse")
                        .build(),
                );
                return;
            }
        };

        if payload.todos.is_empty() {
            return;
        }

        let items: Vec<MergedTodo> = payload
            .todos
            .iter()
            .map(|todo| MergedTodo {
                content: todo.content.clone(),
                completed: todo.status.eq_ignore_ascii_case("completed"),
                // Used only if the entry becomes a new row.
                id: new_task_id(),
            })
            .collect();
        let owners: Vec<String> = cx
            .global::<GlobalTasks>()
            .records()
            .filter(|task| task.session_ids.iter().any(|s| s == session_id))
            .map(|task| task.id().to_owned())
            .collect();
        for id in owners {
            let event = TaskEvent::TodoMerged {
                id,
                session_id: session_id.to_owned(),
                items: items.clone(),
                at: Utc::now(),
            };
            RunsHost::propose_fact(event, cx);
        }
    }
}

/// The execution of a task whose run is `Running`, the only state a
/// session can settle or escalate.
fn running_exec(task: &TaskRecord) -> Option<ExecutionRef> {
    match &task.lifecycle {
        Lifecycle::Running { exec, .. } => Some(exec.clone()),
        _ => None,
    }
}

fn task_kind(cx: &gpui::App, task_id: &str) -> daruda_store::tasks::LifecycleKind {
    cx.global::<GlobalTasks>()
        .get(task_id)
        .map_or(daruda_store::tasks::LifecycleKind::Backlog, |task| {
            task.lifecycle.kind()
        })
}

/// Background loop that drives the Tasks-tab pulse + duration
/// updates. Self-terminates as soon as the global task list contains
/// no `Running` row, so the workspace doesn't burn wakeups while
/// every task is idle.
pub(super) fn spawn_task_live_tick(cx: &mut Context<Workspace>) -> gpui::Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(TASK_LIVE_TICK).await;
            let still_alive = this
                .update(cx, |ws, cx| {
                    let running_exists = cx.global::<GlobalTasks>().any_active();
                    if running_exists {
                        cx.notify();
                        ws.notify_right_dock(cx);
                    }
                    running_exists
                })
                .unwrap_or(false);
            if !still_alive {
                break;
            }
        }
    })
}
