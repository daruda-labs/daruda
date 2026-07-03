//! Right-panel Tasks tab — `Workspace`-side operations.
//!
//! Splits cleanly into three layers:
//! - **Filter / expansion** (R-12) — pure UI state changes.
//! - **CRUD** (R-13) — `create_task` / `update_task` / `delete_task`.
//! - **Lifecycle** (R-14) — `start_task` / `cancel_task` / `reopen_task` /
//!   `retry_task` / `focus_task_lane`.
//!
//! Persistence routes through `save_tasks_dirty`, which always wraps the
//! disk write in `cx.defer` (G9 + `lint-reentrant-reads.sh`) so the
//! background-executor task is queued *after* the current update cycle
//! finishes — never re-entering the workspace entity.

use crate::agent::tasks_global::GlobalTasks;
use crate::ui::dialog::ButtonVariant;
use chrono::Utc;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{BorrowAppContext, Context, Window};

use super::task_picker_modal::{TaskPickAction, TaskPickerModal};
use crate::workspace::Workspace;

impl Workspace {
    // ------------------------------------------------------------------
    // R-12: filter / expansion / persistence
    // ------------------------------------------------------------------

    /// Cycle the Tasks tab filter through `All → Backlog → Running →
    /// Done → All`. Used by the header chip until a `Select` widget
    /// lands.
    pub(in crate::workspace) fn cycle_task_filter(&mut self, cx: &mut Context<Self>) {
        self.task_filter = match self.task_filter {
            daruda_store::tasks::TaskFilter::All => daruda_store::tasks::TaskFilter::Backlog,
            daruda_store::tasks::TaskFilter::Backlog => daruda_store::tasks::TaskFilter::Running,
            daruda_store::tasks::TaskFilter::Running => daruda_store::tasks::TaskFilter::Done,
            daruda_store::tasks::TaskFilter::Done => daruda_store::tasks::TaskFilter::All,
        };
        cx.notify();
    }

    /// Clear the Tasks tab search input (the in-field `✕` overlay).
    /// Extracted so the View closure can dispatch in one line.
    pub(in crate::workspace) fn clear_task_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.task_search_input.clone();
        input.update(cx, |inp, cx_state| {
            inp.set_value("".to_string(), window, cx_state);
        });
        cx.notify();
    }

    /// Set the Tasks-tab filter directly. Called by future `Select`
    /// widget subscriptions once it replaces the cycle chip.
    #[allow(dead_code)]
    pub(in crate::workspace) fn set_task_filter(
        &mut self,
        filter: daruda_store::tasks::TaskFilter,
        cx: &mut Context<Self>,
    ) {
        if self.task_filter != filter {
            self.task_filter = filter;
            cx.notify();
        }
    }

    /// Ensure the Tasks tab's live tick is alive exactly when at least
    /// one task is in the `Running` state (R-23). The tick re-renders
    /// the workspace at [`theme::RIGHT_PANEL_TASK_LIVE_TICK_MS`]
    /// cadence so the pulse dot animates and the inline duration text
    /// advances. When no `Running` task remains, the tick stops by
    /// dropping its `gpui::Task<()>` handle and the workspace burns
    /// zero wakeups while idle.
    ///
    /// Idempotent — replacing the handle when none is alive starts
    /// the loop; replacing it again while it's alive is harmless
    /// (the prior `gpui::Task` is dropped before the new one starts,
    /// mirroring the `_error_expire_sweep` pattern).
    pub(in crate::workspace) fn ensure_task_live_tick(&mut self, cx: &mut Context<Self>) {
        let has_running = cx
            .global::<GlobalTasks>()
            .0
            .tasks
            .iter()
            .any(|t| matches!(t.state, daruda_store::tasks::TaskState::Running { .. }));
        if has_running {
            if self._task_live_tick.is_none() {
                self._task_live_tick =
                    Some(crate::workspace::right_dock::task_workflow_ops::spawn_task_live_tick(cx));
            }
        } else {
            self._task_live_tick = None;
        }
    }

    /// Persist `tasks` to `<data_dir>/tasks.json`. Always defers the
    /// actual `cx.background_executor` spawn so the disk write can
    /// never race with the active update cycle (lint-reentrant-reads).
    ///
    /// Routes through `self.data_dir` rather than
    /// `daruda_store::persistence::default_data_dir()` so tests that
    /// build the workspace with a fresh temp dir don't bleed into the
    /// real `~/Library/Application Support/daruda/` — same pattern
    /// as `main_area::bottom_dock::macro_ops::save_panels`.
    pub(in crate::workspace) fn save_tasks_dirty(&self, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            let weak_for_spawn = weak.clone();
            weak.update(cx, |ws, cx| {
                let snapshot = cx.global::<GlobalTasks>().0.clone();
                let dir = ws.data_dir.clone();
                cx.spawn(async move |_, cx| {
                    let dir_for_async = dir.clone();
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            daruda_store::tasks::persistence::save_tasks_in(
                                &dir_for_async,
                                &snapshot,
                            )
                        })
                        .await;
                    if let Err(e) = result {
                        use daruda_store::observability::system_info::redact_home;
                        let report = ErrorReport::new("Tasks save failed")
                            .severity(ErrorSeverity::Warning)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("dir", redact_home(&dir))
                            .dedup("tasks.save")
                            .build();
                        // `report_error` routes to toast + history + NDJSON;
                        // fall back to a log-only write when the workspace
                        // entity is already gone (window closing).
                        if weak_for_spawn
                            .update(cx, |ws, cx| ws.report_error(report.clone(), cx))
                            .is_err()
                        {
                            use daruda_store::observability::log_writer::LogWriter;
                            LogWriter::log(report);
                        }
                    }
                })
                .detach();
            })
            .ok();
        });
    }

    // ------------------------------------------------------------------
    // R-13: CRUD
    // ------------------------------------------------------------------

    /// Update editable fields on an existing task. The lifecycle state
    /// (`Backlog`, `Running`, …) is intentionally *not* part of the
    /// editable surface — that is driven by the workflow.
    ///
    /// `base_worktree_path = None` maps to "use the project's active
    /// lane at start_task time" (C-1 review note). Re-editing a
    /// `Running` / `Done` task's base has no immediate effect — the
    /// field is consulted only when a Backlog task transitions to
    /// `Running` via `start_task`.
    // Argument list grew once `base_worktree_path` joined the editable
    // surface (C-1). Bundling these into a `TaskUpdate` struct is a
    // worthwhile follow-up but out of scope for the review fix — every
    // call site already passes named locals so the verbosity is
    // bounded.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::workspace) fn update_task(
        &mut self,
        task_id: &str,
        title: String,
        prompt: String,
        notes: String,
        auto_execute: bool,
        agent_surface: daruda_store::tasks::TaskAgentSurface,
        base_worktree_path: Option<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        cx.update_global::<GlobalTasks, _>(|g, _| {
            if let Some(task) = g.get_mut(task_id) {
                task.title = title;
                task.prompt = prompt;
                task.notes = notes;
                task.auto_execute = auto_execute;
                task.agent_surface = agent_surface;
                task.base_worktree_path = base_worktree_path;
                task.updated_at = Utc::now();
            }
        });
        self.save_tasks_dirty(cx);
        cx.notify();
    }

    // ------------------------------------------------------------------
    // R-21: subtask CRUD
    // ------------------------------------------------------------------
    //
    // Mutations are applied through `cx.update_global::<GlobalTasks, _>`
    // and persisted via `save_tasks_dirty` immediately — subtask
    // toggles are explicit user commits (checkbox click / Enter / X),
    // not buffered form edits, so the TaskEdit pane's `saved_snapshot`
    // dirty comparison deliberately ignores subtasks.

    /// Append a manually-added subtask to `task_id`. Empty / whitespace
    /// titles are rejected silently — the inline `[+ Add subtask…]`
    /// input drops empty submits without surfacing an error.
    pub(in crate::workspace) fn add_subtask(
        &mut self,
        task_id: &str,
        title: String,
        cx: &mut Context<Self>,
    ) {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return;
        }
        let trimmed = trimmed.to_string();
        let mutated = cx.update_global::<GlobalTasks, bool>(|g, _| {
            let Some(task) = g.get_mut(task_id) else {
                return false;
            };
            task.subtasks
                .push(daruda_store::tasks::SubTask::new(trimmed));
            task.updated_at = Utc::now();
            true
        });
        if mutated {
            self.save_tasks_dirty(cx);
            cx.notify();
        }
    }

    /// Flip a subtask's `completed` flag. The owning task's
    /// `updated_at` is bumped so list ordering reacts; the subtask's
    /// `source_session_id` is left alone so an "auto" item the user
    /// checks off stays labelled "auto".
    pub(in crate::workspace) fn toggle_subtask(
        &mut self,
        task_id: &str,
        subtask_id: &str,
        cx: &mut Context<Self>,
    ) {
        let mutated = cx.update_global::<GlobalTasks, bool>(|g, _| {
            let Some(task) = g.get_mut(task_id) else {
                return false;
            };
            let Some(sub) = task.subtasks.iter_mut().find(|s| s.id == subtask_id) else {
                return false;
            };
            sub.completed = !sub.completed;
            task.updated_at = Utc::now();
            true
        });
        if mutated {
            self.save_tasks_dirty(cx);
            cx.notify();
        }
    }

    /// Remove a subtask by id. No-op when the task or subtask is gone.
    /// Deletion is immediate with no undo — the X button is the only path.
    pub(in crate::workspace) fn delete_subtask(
        &mut self,
        task_id: &str,
        subtask_id: &str,
        cx: &mut Context<Self>,
    ) {
        let mutated = cx.update_global::<GlobalTasks, bool>(|g, _| {
            let Some(task) = g.get_mut(task_id) else {
                return false;
            };
            let before = task.subtasks.len();
            task.subtasks.retain(|s| s.id != subtask_id);
            if task.subtasks.len() == before {
                return false;
            }
            task.updated_at = Utc::now();
            true
        });
        if mutated {
            self.save_tasks_dirty(cx);
            cx.notify();
        }
    }

    /// Replace a subtask's title. Empty / whitespace titles are
    /// rejected (the inline rename input commits via Enter or blur;
    /// either path drops empty strings rather than deleting the row
    /// silently — destructive removal goes through the X button).
    pub(in crate::workspace) fn rename_subtask(
        &mut self,
        task_id: &str,
        subtask_id: &str,
        new_title: String,
        cx: &mut Context<Self>,
    ) {
        let trimmed = new_title.trim();
        if trimmed.is_empty() {
            return;
        }
        let trimmed = trimmed.to_string();
        let mutated = cx.update_global::<GlobalTasks, bool>(|g, _| {
            let Some(task) = g.get_mut(task_id) else {
                return false;
            };
            let Some(sub) = task.subtasks.iter_mut().find(|s| s.id == subtask_id) else {
                return false;
            };
            if sub.title == trimmed {
                return false;
            }
            sub.title = trimmed;
            task.updated_at = Utc::now();
            true
        });
        if mutated {
            self.save_tasks_dirty(cx);
            cx.notify();
        }
    }

    /// Permanent delete. The associated lane (if any) is *not*
    /// removed — the user takes care of that from the left dock so two
    /// destructive actions never share one click. (D-1)
    pub(in crate::workspace) fn delete_task(&mut self, task_id: &str, cx: &mut Context<Self>) {
        // Drop pending failure tallies before the task itself
        // disappears, otherwise the entries stay forever.
        let stale_sessions: Vec<String> = cx
            .global::<GlobalTasks>()
            .get(task_id)
            .map(|t| t.session_ids.clone())
            .unwrap_or_default();
        for sid in &stale_sessions {
            self.claude.tool_use_failure_counts.remove(sid);
        }
        cx.update_global::<GlobalTasks, _>(|g, _| g.remove(task_id));
        self.save_tasks_dirty(cx);
        cx.notify();
    }

    // ------------------------------------------------------------------
    // R-13 picker (start/cancel/reopen/retry/delete)
    // ------------------------------------------------------------------

    /// Open the [`TaskPickerModal`] for the given action. The five
    /// command-palette entries (Start / Cancel / Reopen / Retry /
    /// Delete) all funnel through here so each action only needs the
    /// state-filter declaration in `TaskPickAction::applies_to` —
    /// adding a sixth action is just one variant + one palette entry.
    pub(in crate::workspace) fn open_task_picker_modal(
        &mut self,
        action: TaskPickAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Build the picker items now, while we still own the
        // `&mut self` borrow. The modal must not re-enter the
        // workspace from inside its constructor.
        let tasks_snapshot = cx.global::<GlobalTasks>().0.clone();
        let items = TaskPickerModal::build_items(&tasks_snapshot, action);
        let weak = cx.weak_entity();
        crate::workspace::dialog_helpers::open_form_modal(
            action.modal_title(),
            None,
            move |window, cx| TaskPickerModal::new(weak.clone(), action, items.clone(), window, cx),
            window,
            cx,
        );
    }

    /// Open an OK-only alert dialog showing the full
    /// `TaskState::Error.message` for `task_id`. No-op for tasks that
    /// are not currently in the `Error` state — the menu entry is
    /// only exposed on Error rows, but defensive guarding keeps the
    /// dispatcher honest if a race lets the state flip between menu
    /// open and click. (R-26 View error.)
    pub(in crate::workspace) fn open_task_error_dialog(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = {
            let g = cx.global::<GlobalTasks>();
            let Some(task) = g.get(task_id) else {
                return;
            };
            match &task.state {
                daruda_store::tasks::TaskState::Error { message, .. } => message.clone(),
                _ => return,
            }
        };
        crate::workspace::dialog_helpers::open_alert_dialog(
            crate::surface::strings::task_error_dialog_title(),
            message,
            crate::surface::strings::task_error_dialog_close(),
            window,
            cx,
        );
    }

    /// Show a Danger-styled `ConfirmModal` before invoking
    /// [`Workspace::delete_task`]. Routed to from the row's `[Delete]`
    /// button and the palette's `Delete Task` entry.
    pub(in crate::workspace) fn open_delete_task_confirm(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let body = {
            let g = cx.global::<GlobalTasks>();
            let Some(task) = g.get(task_id) else {
                return;
            };
            crate::surface::strings::task_confirm_delete_body(&task.title)
        };
        let weak = cx.weak_entity();
        let id = task_id.to_string();
        crate::workspace::dialog_helpers::open_confirm_dialog(
            crate::surface::strings::task_picker_title_delete(),
            body,
            crate::surface::strings::task_action_delete(),
            ButtonVariant::Danger,
            move |_, _window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    let id = id.clone();
                    ws.update(app_cx, |ws, cx| ws.delete_task(&id, cx));
                }
            },
            window,
            cx,
        );
    }
}
