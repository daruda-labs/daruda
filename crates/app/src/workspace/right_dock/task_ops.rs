//! Right-panel Tasks tab — `Workspace`-side operations.
//!
//! Splits cleanly into three layers:
//! - **Filter / expansion** — pure UI state changes.
//! - **CRUD** — `create_task` / `update_task` / `delete_task`.
//! - **Lifecycle** — `start_task` / `cancel_task` / `reopen_task` /
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

/// The form-editable fields of a saved task, as `update_task` applies them.
pub(in crate::workspace) struct TaskEdits {
    pub(in crate::workspace) title: String,
    pub(in crate::workspace) prompt: String,
    pub(in crate::workspace) notes: String,
    pub(in crate::workspace) auto_execute: bool,
    pub(in crate::workspace) agent_surface: daruda_store::tasks::TaskAgentSurface,
    pub(in crate::workspace) base_worktree_path: Option<std::path::PathBuf>,
    /// Trimmed and validated; `None` keeps the current branch.
    pub(in crate::workspace) branch: Option<String>,
    /// `None` keeps where the task runs.
    pub(in crate::workspace) run_in: Option<daruda_store::tasks::TaskRunIn>,
    /// `None` keeps the task's project.
    pub(in crate::workspace) project: Option<daruda_store::project::ProjectUuid>,
}

impl Workspace {
    // ------------------------------------------------------------------
    // Filter / expansion / persistence
    // ------------------------------------------------------------------

    pub(in crate::workspace) fn set_task_grouping(
        &mut self,
        mode: super::tasks::TaskGrouping,
        cx: &mut Context<Self>,
    ) {
        self.task_browser.state.groups.set_mode(mode);
        cx.notify();
    }

    pub(in crate::workspace) fn toggle_task_group(
        &mut self,
        key: super::tasks::TaskGroupKey,
        cx: &mut Context<Self>,
    ) {
        self.task_browser.state.groups.toggle(key);
        cx.notify();
    }

    /// Keep matching filters; relax only those hiding the saved task.
    pub(in crate::workspace) fn show_saved_task(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = cx.global::<GlobalTasks>().get(id) else {
            return;
        };
        let active = self.active_project().map(|project| project.uuid);
        self.task_browser.state.reveal(task, active);
        let query = self
            .task_browser
            .search
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        if !super::tasks::matches_task(task, &query) {
            self.clear_task_search(window, cx);
        }
        self.open_page(crate::workspace::pages::Page::Tasks, window, cx);
    }

    pub(in crate::workspace) fn set_task_scope(
        &mut self,
        scope: daruda_store::tasks::TaskScope,
        cx: &mut Context<Self>,
    ) {
        self.task_browser.state.scope = scope;
        cx.notify();
    }

    pub(in crate::workspace) fn new_task_in_scope(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.active_project().map(|p| p.uuid);
        let project = self.task_browser.state.scope.project(active).or(active);
        if let Some(project) = project.filter(|id| self.project_by_uuid(*id).is_some()) {
            self.open_task_draft_for_project(project, window, cx);
        }
    }

    pub(in crate::workspace) fn clear_task_filters(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.task_browser.state.filter = daruda_store::tasks::TaskFilter::All;
        self.clear_task_search(window, cx);
    }

    /// Clear the Tasks tab search input (the in-field `✕` overlay).
    /// Extracted so the View closure can dispatch in one line.
    pub(super) fn clear_task_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.task_browser.search.clone();
        input.update(cx, |inp, cx_state| {
            inp.set_value("".to_string(), window, cx_state);
        });
        cx.notify();
    }

    /// Select a status without changing the project or search query.
    pub(in crate::workspace) fn set_task_filter(
        &mut self,
        filter: daruda_store::tasks::TaskFilter,
        cx: &mut Context<Self>,
    ) {
        if self.task_browser.state.filter != filter {
            self.task_browser.state.filter = filter;
            cx.notify();
        }
    }

    /// Ensure the Tasks tab's live tick is alive exactly when at least
    /// one task is in the `Running` state. The tick re-renders
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
            if self.pumps.task_live_tick.is_none() {
                self.pumps.task_live_tick =
                    Some(crate::workspace::right_dock::task_workflow_ops::spawn_task_live_tick(cx));
            }
        } else {
            self.pumps.task_live_tick = None;
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
        if self.persistence_suspended {
            return;
        }
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
                        let report =
                            ErrorReport::new(crate::surface::strings::error::tasks_save_failed())
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
    // CRUD
    // ------------------------------------------------------------------

    /// Update editable fields on an existing task. The lifecycle state
    /// (`Backlog`, `Running`, …) is intentionally *not* part of the
    /// editable surface — that is driven by the workflow.
    ///
    /// Where the task runs is only consulted when a Backlog task starts, so
    /// `edits.branch` applies to a Backlog task alone; once started, the
    /// branch names the lane it created and stays fixed.
    pub(in crate::workspace) fn update_task(
        &mut self,
        task_id: &str,
        edits: TaskEdits,
        cx: &mut Context<Self>,
    ) {
        cx.update_global::<GlobalTasks, _>(|g, _| {
            if let Some(task) = g.get_mut(task_id) {
                task.title = edits.title;
                task.prompt = edits.prompt;
                task.notes = edits.notes;
                task.auto_execute = edits.auto_execute;
                task.agent_surface = edits.agent_surface;
                task.base_worktree_path = edits.base_worktree_path;
                if matches!(task.state, daruda_store::tasks::TaskState::Backlog) {
                    if let Some(branch) = edits.branch {
                        task.branch_name = branch;
                    }
                    if let Some(run_in) = edits.run_in {
                        task.run_in = run_in;
                    }
                    if let Some(project) = edits.project {
                        task.project = project;
                    }
                }
                task.updated_at = Utc::now();
            }
        });
        self.save_tasks_dirty(cx);
        cx.notify();
    }

    // ------------------------------------------------------------------
    // Subtask CRUD
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
    // Lifecycle picker (start/cancel/reopen/retry/delete)
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
    /// open and click.
    pub(super) fn open_task_error_dialog(
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
            crate::surface::strings::task::error_dialog_title(),
            message,
            crate::surface::strings::common::btn_close(),
            window,
            cx,
        );
    }

    /// Show a Danger-styled `ConfirmModal` before invoking
    /// [`Workspace::delete_task`]. Routed to from the row's `[Delete]`
    /// button and the palette's `Delete Task` entry.
    pub(super) fn open_delete_task_confirm(
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
            crate::surface::strings::task::confirm_delete_body(&task.title)
        };
        let weak = cx.weak_entity();
        let id = task_id.to_string();
        crate::workspace::dialog_helpers::open_confirm_dialog(
            crate::surface::strings::task::picker_title_delete(),
            body,
            crate::surface::strings::common::btn_delete(),
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
