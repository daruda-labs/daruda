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

use crate::agent::tasks_global::GlobalTasks;
use chrono::Utc;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::tasks::SubTask;
use gpui::{BorrowAppContext, Context, Window};
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

    /// Direct PTY write into a specific pane by id. Returns `false`
    /// when the id no longer exists (pane closed mid-flight) or the
    /// pane isn't a terminal. Targeting by id — not by `focused_pane_id`
    /// — guarantees the bytes land in the pane the caller intends,
    /// which matters for task dispatch where focus may shift between
    /// `finalize_create_lane` and the actual write.
    pub(super) fn send_to_pane(
        &self,
        pane_id: crate::workspace::main_area::pane_tree::PaneId,
        bytes: &[u8],
    ) -> bool {
        self.active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .map(|p| p.send_input(bytes))
            .unwrap_or(false)
    }

    /// User-initiated stop. Lane is preserved (D-1).
    pub(in crate::workspace) fn cancel_task(&mut self, task_id: &str, cx: &mut Context<Self>) {
        let cleared_sessions = cx.update_global::<GlobalTasks, Vec<String>>(|g, _| {
            if let Some(task) = g.get_mut(task_id)
                && let Some(wt) = task.state.worktree_path().cloned()
            {
                let ids = std::mem::take(&mut task.session_ids);
                task.state = daruda_store::tasks::TaskState::Cancelled { worktree_path: wt };
                task.finished_at = Some(Utc::now());
                task.updated_at = Utc::now();
                ids
            } else {
                Vec::new()
            }
        });
        // Drop pending PostToolUseFailure tallies — ULIDs are never
        // reused, so leaving them would slowly accumulate dead
        // entries across many cancel/reopen cycles.
        for sid in &cleared_sessions {
            self.claude.tool_use_failure_counts.remove(sid);
        }
        self.cancel_task_chat_execution(task_id, cx);
        self.save_tasks_dirty(cx);
        cx.notify();
    }

    /// Switch the active lane to the one this task is running in.
    /// Lazily transitions to `Error { "lane gone" }` when the
    /// path no longer exists on disk, so a deleted-from-the-
    /// outside checkout doesn't dangle in `Running` forever.
    pub(in crate::workspace) fn focus_task_lane(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = cx
            .global::<GlobalTasks>()
            .get(task_id)
            .and_then(|t| t.state.worktree_path().cloned())
        else {
            return;
        };

        if !path.exists() {
            let path_for_state = path.clone();
            let cleared = cx.update_global::<GlobalTasks, Vec<String>>(|g, _| {
                if let Some(t) = g.get_mut(task_id) {
                    let ids = std::mem::take(&mut t.session_ids);
                    t.state = daruda_store::tasks::TaskState::Error {
                        worktree_path: path_for_state,
                        message: crate::surface::strings::task::error_lane_gone(),
                    };
                    t.updated_at = Utc::now();
                    ids
                } else {
                    Vec::new()
                }
            });
            for sid in &cleared {
                self.claude.tool_use_failure_counts.remove(sid);
            }
            self.save_tasks_dirty(cx);
            cx.notify();
            return;
        }

        let target_id = self
            .active_lanes()
            .iter()
            .find(|w| w.path == path)
            .map(|w| w.id);
        if let Some(id) = target_id {
            let target = daruda_store::project::LaneRef {
                project: self.active.project,
                lane: id,
            };
            self.activate_lane(target, window, cx);
        }
    }

    /// Move a terminal-state task back to `Backlog`; the next [Start] runs
    /// it again (see `return_task_to_backlog` for where).
    pub(in crate::workspace) fn reopen_task(&mut self, task_id: &str, cx: &mut Context<Self>) {
        self.return_task_to_backlog(task_id, cx);
    }

    /// `Reopen` + `start_task` in one click — for the `[Retry]`
    /// affordance on `Error` rows.
    pub(super) fn retry_task(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.return_task_to_backlog(task_id, cx);
        self.start_task(task_id, window, cx);
    }

    /// Put a finished task back in Backlog so it can run again — in the lane
    /// its earlier run created, while that lane is still registered. Saved
    /// here, since a Retry's Start may stop before it saves anything.
    fn return_task_to_backlog(&mut self, task_id: &str, cx: &mut Context<Self>) {
        let own_lane = cx
            .global::<GlobalTasks>()
            .get(task_id)
            .and_then(|task| self.own_lane_for_rerun(task));
        let cleared = cx.update_global::<GlobalTasks, Vec<String>>(|g, _| {
            let Some(task) = g.get_mut(task_id) else {
                return Vec::new();
            };
            if let Some(path) = own_lane {
                task.run_in = daruda_store::tasks::TaskRunIn::ExistingLane { path };
            }
            task.state = daruda_store::tasks::TaskState::Backlog;
            task.finished_at = None;
            task.updated_at = Utc::now();
            std::mem::take(&mut task.session_ids)
        });
        for sid in &cleared {
            self.claude.tool_use_failure_counts.remove(sid);
        }
        self.save_tasks_dirty(cx);
        cx.notify();
    }

    // ------------------------------------------------------------------
    // Claude hook → task state mapping
    // ------------------------------------------------------------------

    /// Attach `session_id` to the `Running` task whose CLI run owns it.
    /// Idempotent — duplicate registrations are skipped so the same hook
    /// firing twice doesn't grow the `session_ids` vec without bound.
    pub(in crate::workspace) fn apply_task_session_changed(
        &mut self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) {
        let dirty = cx.update_global::<GlobalTasks, bool>(|g, _| {
            let mut dirty = false;
            for task in g.tasks.iter_mut() {
                if !matches!(task.state, daruda_store::tasks::TaskState::Running { .. }) {
                    continue;
                }
                if task.agent_surface != daruda_store::tasks::TaskAgentSurface::Terminal {
                    continue;
                }
                if !task.execution.as_ref().is_some_and(|run| {
                    run.source.cli_process().is_some()
                        && run.session_id.as_deref() == Some(session_id)
                }) {
                    continue;
                }
                if !task.session_ids.iter().any(|s| s == session_id) {
                    task.session_ids.push(session_id.to_string());
                    task.updated_at = Utc::now();
                    dirty = true;
                }
            }
            dirty
        });
        if dirty {
            self.save_tasks_dirty(cx);
            cx.notify();
        }
    }

    /// Transition every `Running` task that owns `session_id` into
    /// `Done` / `Error`, depending on `reason`. The `session_ids`
    /// list is cleared on transition so a future `Retry` starts clean.
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
        self.claude.tool_use_failure_counts.remove(session_id);

        let dirty = cx.update_global::<GlobalTasks, bool>(|g, _| {
            let mut dirty = false;
            for task in g.tasks.iter_mut() {
                if task.agent_surface != daruda_store::tasks::TaskAgentSurface::Terminal {
                    continue;
                }
                if !task.session_ids.iter().any(|s| s == session_id) {
                    continue;
                }
                if let daruda_store::tasks::TaskState::Running { worktree_path } =
                    task.state.clone()
                {
                    task.state = match reason {
                        daruda_store::tasks::SessionEndReason::Error => {
                            daruda_store::tasks::TaskState::Error {
                                worktree_path,
                                message: crate::surface::strings::task::error_session_failed(),
                            }
                        }
                        other => daruda_store::tasks::TaskState::Done {
                            worktree_path,
                            end_reason: other,
                        },
                    };
                    task.finished_at = Some(Utc::now());
                    task.updated_at = Utc::now();
                    dirty = true;
                }
                // `retain` runs even on already-terminal tasks. That's
                // intentional: a stale session_id from a long-completed
                // task should still be scrubbed if we somehow see one,
                // and the operation is a no-op when the vec is already
                // empty (cancel/reopen path drained it).
                task.session_ids.retain(|s| s != session_id);
            }
            dirty
        });
        if dirty {
            self.save_tasks_dirty(cx);
            cx.notify();
        }
    }

    /// Only the pane that dispatched this execution may settle its task.
    /// Restoring history or a stale execution's late event owns no new work.
    pub(in crate::workspace) fn apply_agent_chat_task_ended(
        &mut self,
        pane_id: crate::workspace::main_area::pane_tree::PaneId,
        reason: daruda_store::tasks::SessionEndReason,
        cx: &mut Context<Self>,
    ) {
        let Some(owner) = self.task_chat_owner(pane_id, cx) else {
            return;
        };
        let dirty = cx.update_global::<GlobalTasks, bool>(|g, _| {
            let mut dirty = false;
            for task in g.tasks.iter_mut() {
                if task.id != owner {
                    continue;
                }
                let daruda_store::tasks::TaskState::Running { worktree_path } = task.state.clone()
                else {
                    continue;
                };
                task.state = match reason {
                    daruda_store::tasks::SessionEndReason::Error => {
                        daruda_store::tasks::TaskState::Error {
                            worktree_path,
                            message: crate::surface::strings::task::error_session_failed(),
                        }
                    }
                    other => daruda_store::tasks::TaskState::Done {
                        worktree_path,
                        end_reason: other,
                    },
                };
                task.finished_at = Some(Utc::now());
                task.updated_at = Utc::now();
                dirty = true;
            }
            dirty
        });
        if dirty {
            self.save_tasks_dirty(cx);
            cx.notify();
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
            .claude
            .tool_use_failure_counts
            .entry(session_id.to_string())
            .and_modify(|n| *n = n.saturating_add(1))
            .or_insert(1);
        let count = *count;
        if count >= daruda_store::tasks::TASK_TOOL_USE_FAILURE_THRESHOLD {
            self.claude.tool_use_failure_counts.remove(session_id);
            let message = crate::surface::strings::task::error_tool_use_failure(count);
            self.escalate_task_session_to_error(session_id, message, cx);
        }
    }

    /// Does any `Running` task carry `session_id` in its `session_ids`?
    /// Used as a guard before incrementing the tool-use-failure counter
    /// so sessions outside daruda's task system don't trigger
    /// escalation.
    fn session_owned_by_running_task(session_id: &str, cx: &Context<Self>) -> bool {
        cx.global::<GlobalTasks>().tasks.iter().any(|task| {
            matches!(task.state, daruda_store::tasks::TaskState::Running { .. })
                && task.agent_surface == daruda_store::tasks::TaskAgentSurface::Terminal
                && task.session_ids.iter().any(|s| s == session_id)
        })
    }

    /// Force every `Running` task that owns `session_id` into
    /// `Error { message }`. Distinct from
    /// `apply_task_session_ended(_, Error, _)` because the message
    /// reflects *why* daruda decided to escalate (e.g.
    /// `tool_use_failure x5`) rather than the generic "session error"
    /// fallback.
    pub(super) fn escalate_task_session_to_error(
        &mut self,
        session_id: &str,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let (matched, dirty) = cx.update_global::<GlobalTasks, (bool, bool)>(|g, _| {
            let mut matched = false;
            let mut dirty = false;
            for task in g.tasks.iter_mut() {
                if !task.session_ids.iter().any(|s| s == session_id) {
                    continue;
                }
                matched = true;
                if let daruda_store::tasks::TaskState::Running { worktree_path } =
                    task.state.clone()
                {
                    task.state = daruda_store::tasks::TaskState::Error {
                        worktree_path,
                        message: message.clone(),
                    };
                    task.finished_at = Some(Utc::now());
                    task.updated_at = Utc::now();
                    dirty = true;
                }
                task.session_ids.retain(|s| s != session_id);
            }
            (matched, dirty)
        });
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
        if dirty {
            self.save_tasks_dirty(cx);
            cx.notify();
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

        let dirty = cx.update_global::<GlobalTasks, bool>(|g, _| {
            let now = Utc::now();
            let mut dirty = false;
            for task in g.tasks.iter_mut() {
                if !task.session_ids.iter().any(|s| s == session_id) {
                    continue;
                }
                for incoming in &payload.todos {
                    let trimmed = incoming.content.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let completed = incoming.status.eq_ignore_ascii_case("completed");
                    // Namespace match: only fold into an existing
                    // subtask when both the title AND the originating
                    // session id agree. This keeps manual rows (None)
                    // off-limits and isolates multiple Claude sessions
                    // sharing the same task.
                    if let Some(existing) = task.subtasks.iter_mut().find(|s| {
                        s.source_session_id.as_deref() == Some(session_id) && s.title == trimmed
                    }) {
                        if existing.completed != completed {
                            existing.completed = completed;
                            dirty = true;
                        }
                    } else {
                        let mut new_sub = SubTask::new(trimmed.to_string());
                        new_sub.completed = completed;
                        new_sub.created_at = Some(now);
                        new_sub.source_session_id = Some(session_id.to_string());
                        task.subtasks.push(new_sub);
                        dirty = true;
                    }
                }
                if dirty {
                    task.updated_at = now;
                }
            }
            dirty
        });

        if dirty {
            self.save_tasks_dirty(cx);
            cx.notify();
        }
    }
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
                    let running_exists =
                        cx.global::<GlobalTasks>().0.tasks.iter().any(|t| {
                            matches!(t.state, daruda_store::tasks::TaskState::Running { .. })
                        });
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
