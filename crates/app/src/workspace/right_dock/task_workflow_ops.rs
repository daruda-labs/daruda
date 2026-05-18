//! Right-panel Tasks tab — R-14 lifecycle layer.
//!
//! Hosts the long-running task workflows: `start_task` (open the
//! worktree → write the prompt file → dispatch `claude` into the
//! new pane's PTY), `cancel_task`, `focus_task_worktree`,
//! `reopen_task`, `retry_task`, plus the supporting Claude-Code
//! session bookkeeping (`apply_task_session_changed`,
//! `apply_task_session_ended`, `apply_todo_write`, …).
//!
//! Sibling of `task_ops` (R-12 filter + R-13 CRUD). Both modules
//! extend the same `Workspace` via separate `impl Workspace` blocks.

use std::path::Path;
use std::time::Duration;

use crate::agent::tasks_global::GlobalTasks;
use crate::ui::theme;
use chrono::Utc;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::tasks::SubTask;
use gpui::{BorrowAppContext, Context, Window};
use serde::Deserialize;

use crate::workspace::Workspace;
use crate::workspace::worktree_ops::CreateWorktreePlan;

/// Subset of the `TodoWrite` tool's `tool_input.todos[]` shape (R-22).
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
    // R-14: lifecycle (start / cancel / focus / reopen / retry)
    // ------------------------------------------------------------------

    /// Per-repo lock acquisition. Two concurrent `start_task` calls
    /// against the same repo race on `git worktree add`; we make the
    /// second one fail fast with a user-visible error rather than
    /// risk a half-created worktree.
    fn acquire_repo_lock(&mut self, repo_root: &Path) -> bool {
        self.pending_worktree_creates
            .insert(repo_root.to_path_buf())
    }

    fn release_repo_lock(&mut self, repo_root: &Path) {
        self.pending_worktree_creates.remove(repo_root);
    }

    /// Resolve the branch name we should base the new worktree on.
    /// Looks the path up in the workspace's worktree list and returns
    /// `worktree.branch.clone()` when it's a git worktree. `None`
    /// falls through to git's "branch from current HEAD" default.
    fn branch_for_worktree_path(&self, path: &Path) -> Option<String> {
        self.active_worktrees()
            .iter()
            .find(|w| w.path == path)
            .and_then(|w| match &w.kind {
                daruda_store::project::WorktreeKind::Git { branch, .. } => branch.clone(),
                _ => None,
            })
    }

    /// Direct PTY write into a specific pane by id. Returns `false`
    /// when the id no longer exists (pane closed mid-flight) or the
    /// pane isn't a terminal. Targeting by id — not by `focused_pane_id`
    /// — guarantees the bytes land in the pane the caller intends,
    /// which matters for task dispatch where focus may shift between
    /// `finalize_create_worktree` and the actual write.
    pub(in crate::workspace) fn send_to_pane(
        &self,
        pane_id: crate::workspace::main_area::pane_tree::PaneId,
        bytes: &[u8],
    ) -> bool {
        self.main_area
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .map(|p| p.send_input(bytes))
            .unwrap_or(false)
    }

    /// Start a Backlog task: open the worktree (`git worktree add`),
    /// then write the prompt file and dispatch the `claude` command
    /// into the new pane's PTY. Mirrors superset-desktop's
    /// `agent-command.ts` + `terminal-adapter.ts` flow but in a
    /// single GPUI spawn.
    pub(in crate::workspace) fn start_task(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Snapshot every read off `self` we'll need inside the spawn —
        // the workspace isn't borrowable across `await` points.
        let Some(task) = cx.global::<GlobalTasks>().get(task_id).cloned() else {
            return;
        };
        if !matches!(task.state, daruda_store::tasks::TaskState::Backlog) {
            return;
        }
        let Some(repo_root) = self.git_repo_root() else {
            let report = ErrorReport::new("Tasks require a git repo")
                .severity(ErrorSeverity::Info)
                .at(file!(), line!())
                .dedup("tasks.no_git_repo")
                .build();
            self.report_error(report, cx);
            return;
        };
        if !self.acquire_repo_lock(&repo_root) {
            let report = ErrorReport::new("Another worktree is already being created")
                .severity(ErrorSeverity::Info)
                .at(file!(), line!())
                .dedup("worktree.create.busy")
                .build();
            self.report_error(report, cx);
            return;
        }

        let repo_name = repo_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project");
        let new_path = repo_root
            .parent()
            .unwrap_or(&repo_root)
            .join(format!("{repo_name}-{}", task.branch_name));

        let base_ref = task
            .base_worktree_path
            .as_deref()
            .and_then(|p| self.branch_for_worktree_path(p));

        let plan = CreateWorktreePlan {
            branch: task.branch_name.clone(),
            new_path: new_path.clone(),
            repo_root: repo_root.clone(),
            base_ref,
            description: Some(format!("task: {}", task.title)),
        };

        let task_id = task.id.clone();
        let me = cx.weak_entity();
        let plan_clone = plan.clone();

        window
            .spawn(cx, async move |async_cx| {
                let result: Result<(), String> = async_cx
                    .background_executor()
                    .spawn({
                        let plan = plan_clone.clone();
                        async move {
                            crate::worktree::git::add_worktree(
                                &plan.repo_root,
                                &plan.new_path,
                                Some(&plan.branch),
                                plan.base_ref.as_deref(),
                            )
                            .map_err(|e| e.to_string())
                        }
                    })
                    .await;

                let _ = async_cx.update(|window, app_cx| {
                    let Some(workspace) = me.upgrade() else {
                        return;
                    };
                    workspace.update(app_cx, |ws, cx| {
                        ws.release_repo_lock(&plan.repo_root);
                        // Lock is released *before* finalize_create_worktree
                        // runs. If finalize fails the new git worktree is
                        // already on disk — daruda just doesn't know about
                        // it. The user can clean up via `git worktree prune`
                        // or remove via the left dock (D-1). The released
                        // lock is intentional so a user retry against the
                        // same repo isn't blocked by a half-created entry.
                        match result {
                            Err(msg) => {
                                let report = ErrorReport::new("Worktree create failed")
                                    .severity(ErrorSeverity::Error)
                                    .at(file!(), line!())
                                    .with_context("detail", msg)
                                    .dedup("worktree.create")
                                    .build();
                                ws.report_error(report, cx);
                            }
                            Ok(()) => match ws.finalize_create_worktree(plan.clone(), window, cx) {
                                Err(msg) => {
                                    let report = ErrorReport::new("Worktree finalize failed")
                                        .severity(ErrorSeverity::Error)
                                        .at(file!(), line!())
                                        .with_context("detail", msg)
                                        .dedup("worktree.create")
                                        .build();
                                    ws.report_error(report, cx);
                                }
                                Ok(pane_id) => {
                                    ws.dispatch_claude_for_task(
                                        &task_id,
                                        &plan.new_path,
                                        pane_id,
                                        window,
                                        cx,
                                    );
                                }
                            },
                        }
                    });
                });
            })
            .detach();
    }

    /// Write `<worktree>/.daruda/task-<branch>.md` and dispatch the
    /// `claude --dangerously-skip-permissions "$(cat '...')"` command
    /// into the freshly-created pane. Marks the task `Running` on
    /// success, `Error` on prompt-file write failure.
    ///
    /// Takes `pane_id` rather than relying on `focused_pane_id` —
    /// `activate_worktree` does call `focus_pane` on the happy path,
    /// but routing the command through the pane the worktree spawned
    /// is bug-resistant against any future change to focus handling.
    fn dispatch_claude_for_task(
        &mut self,
        task_id: &str,
        worktree_path: &Path,
        pane_id: crate::workspace::main_area::pane_tree::PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = cx.global::<GlobalTasks>().get(task_id).cloned() else {
            return;
        };
        let rendered = daruda_store::tasks::prompt_file::render_task_prompt(&task);
        let prompt_path = match daruda_store::tasks::prompt_file::write_prompt_file(
            worktree_path,
            &task.branch_name,
            &rendered,
        ) {
            Ok(p) => p,
            Err(e) => {
                cx.update_global::<GlobalTasks, _>(|g, _| {
                    if let Some(t) = g.get_mut(task_id) {
                        t.state = daruda_store::tasks::TaskState::Error {
                            worktree_path: worktree_path.to_path_buf(),
                            message: format!("write prompt: {e}"),
                        };
                        t.updated_at = Utc::now();
                    }
                });
                self.save_tasks_dirty(cx);
                cx.notify();
                return;
            }
        };
        let cmd =
            daruda_store::tasks::prompt_file::build_claude_command(&prompt_path, task.auto_execute);
        self.send_to_pane(pane_id, cmd.as_bytes());

        cx.update_global::<GlobalTasks, _>(|g, _| {
            if let Some(t) = g.get_mut(task_id) {
                t.state = daruda_store::tasks::TaskState::Running {
                    worktree_path: worktree_path.to_path_buf(),
                };
                t.updated_at = Utc::now();
            }
        });
        self.save_tasks_dirty(cx);

        // R-20 dynamic install: if a TaskEdit pane for this task is
        // already open (the user clicked Start from the pane footer),
        // the prompt file just landed on disk for the first time —
        // install the FS watcher now instead of waiting for the user
        // to close-and-reopen the pane.
        self.attach_prompt_watcher_if_pane_open(task_id, window, cx);

        cx.notify();
    }

    /// User-initiated stop. Worktree is preserved (D-1).
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
        self.save_tasks_dirty(cx);
        cx.notify();
    }

    /// Switch the active worktree to the one this task is running in.
    /// Lazily transitions to `Error { "worktree gone" }` when the
    /// path no longer exists on disk (D-10), so a deleted-from-the-
    /// outside checkout doesn't dangle in `Running` forever.
    pub(in crate::workspace) fn focus_task_worktree(
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
                        message: "worktree gone".into(),
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
            .active_worktrees()
            .iter()
            .find(|w| w.path == path)
            .map(|w| w.id);
        if let Some(id) = target_id {
            let target = daruda_store::project::WorktreeRef {
                project: self.active.project,
                worktree: id,
            };
            self.activate_worktree(target, window, cx);
        }
    }

    /// Move a terminal-state task back to `Backlog`. Worktree path is
    /// dropped from `Task::state`; user must press [Start] to spawn
    /// a fresh worktree.
    pub(in crate::workspace) fn reopen_task(&mut self, task_id: &str, cx: &mut Context<Self>) {
        let cleared = cx.update_global::<GlobalTasks, Vec<String>>(|g, _| {
            if let Some(task) = g.get_mut(task_id) {
                let ids = std::mem::take(&mut task.session_ids);
                task.state = daruda_store::tasks::TaskState::Backlog;
                task.finished_at = None;
                task.updated_at = Utc::now();
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
    }

    /// `Reopen` + `start_task` in one click — for the `[Retry]`
    /// affordance on `Error` rows.
    pub(in crate::workspace) fn retry_task(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cleared = cx.update_global::<GlobalTasks, Vec<String>>(|g, _| {
            if let Some(task) = g.get_mut(task_id) {
                let ids = std::mem::take(&mut task.session_ids);
                task.state = daruda_store::tasks::TaskState::Backlog;
                task.updated_at = Utc::now();
                ids
            } else {
                Vec::new()
            }
        });
        for sid in &cleared {
            self.claude.tool_use_failure_counts.remove(sid);
        }
        self.start_task(task_id, window, cx);
    }

    // ------------------------------------------------------------------
    // R-15: Claude hook → task state mapping
    // ------------------------------------------------------------------

    /// Attach `session_id` to every `Running` task whose worktree
    /// matches the hook's `cwd`. Idempotent — duplicate registrations
    /// are skipped so the same hook firing twice doesn't grow the
    /// `session_ids` vec without bound.
    pub(in crate::workspace) fn apply_task_session_changed(
        &mut self,
        cwd: &Path,
        session_id: &str,
        cx: &mut Context<Self>,
    ) {
        let dirty = cx.update_global::<GlobalTasks, bool>(|g, _| {
            let mut dirty = false;
            for task in g.tasks.iter_mut() {
                let daruda_store::tasks::TaskState::Running { worktree_path } = &task.state else {
                    continue;
                };
                if worktree_path != cwd {
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
                                message: "session error".into(),
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
            let message = format!("tool_use_failure x{count}");
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
                && task.session_ids.iter().any(|s| s == session_id)
        })
    }

    /// Force every `Running` task that owns `session_id` into
    /// `Error { message }`. Distinct from
    /// `apply_task_session_ended(_, Error, _)` because the message
    /// reflects *why* daruda decided to escalate (e.g.
    /// `tool_use_failure x5`) rather than the generic "session error"
    /// fallback.
    pub(in crate::workspace) fn escalate_task_session_to_error(
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
            let report = ErrorReport::new("Tool-use-failure escalation has no matching task")
                .severity(ErrorSeverity::Warning)
                .message(format!(
                    "session {session_id} hit the escalation threshold but no Running task owns it ({message})"
                ))
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
    // R-22: TodoWrite hook auto-merge
    // ------------------------------------------------------------------

    /// Merge a `TodoWrite` hook payload into the matching task's
    /// subtasks (R-22).
    ///
    /// Routing: the workspace already tracks which Claude session id
    /// belongs to which `Running` task via `session_ids` (R-15). We
    /// reuse that linkage to find the target task — no extra
    /// bookkeeping needed.
    ///
    /// Policy (I-14 namespace isolation):
    /// - User-added subtasks (`source_session_id == None`) are
    ///   **never** touched, even if a hook todo carries the same title.
    /// - For matches against an existing auto-subtask we only flip
    ///   `completed`; titles aren't rewritten (Claude sometimes shifts
    ///   wording across emissions and we don't want the row text to
    ///   jitter under the user's cursor).
    /// - Unknown titles for this session_id are pushed as new auto
    ///   rows. Items Claude *drops* on subsequent emissions stay put
    ///   (rows dropped by Claude on subsequent emissions stay put — no stale marking).
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
pub(in crate::workspace) fn spawn_task_live_tick(cx: &mut Context<Workspace>) -> gpui::Task<()> {
    let interval = Duration::from_millis(theme::RIGHT_PANEL_TASK_LIVE_TICK_MS);
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(interval).await;
            let still_alive = this
                .update(cx, |_ws, cx| {
                    let running_exists =
                        cx.global::<GlobalTasks>().0.tasks.iter().any(|t| {
                            matches!(t.state, daruda_store::tasks::TaskState::Running { .. })
                        });
                    if running_exists {
                        cx.notify();
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
