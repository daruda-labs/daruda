//! Right-panel Tasks tab — `Workspace`-side operations.
//!
//! Splits cleanly into three layers:
//! - **Filter / expansion** — pure UI state changes.
//! - **CRUD** — `create_task` / `update_task` / `delete_task`.
//! - **Lifecycle** — `start_task` / `cancel_task` / `reopen_task` /
//!   `retry_task` / `focus_task_lane`.
//!
//! Every change is a `TaskEvent` proposed to `RunsHost`; the projection
//! the views read changes only when the log commits it.

use crate::runs::GlobalTasks;
use crate::runs::TaskIntent;
use crate::ui::dialog::ButtonVariant;
use daruda_store::tasks::SubTask;
use daruda_store::tasks::machine::{LifecycleKind, TaskEdit};
use gpui::{Context, Window};

use super::task_picker_modal::{TaskPickAction, TaskPickerModal};
use crate::workspace::Workspace;

impl Workspace {
    // ------------------------------------------------------------------
    // Filter / expansion / persistence
    // ------------------------------------------------------------------

    pub(in crate::workspace) fn set_task_grouping(
        &mut self,
        mode: super::tasks::TaskGrouping,
        cx: &mut Context<Self>,
    ) {
        self.right_views.tasks.state.groups.set_mode(mode);
        cx.notify();
    }

    pub(in crate::workspace) fn toggle_task_group(
        &mut self,
        key: super::tasks::TaskGroupKey,
        cx: &mut Context<Self>,
    ) {
        self.right_views.tasks.state.groups.toggle(key);
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
        self.right_views.tasks.state.reveal(task, active);
        let query = self
            .right_views
            .tasks
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
        self.right_views.tasks.state.scope = scope;
        cx.notify();
    }

    pub(in crate::workspace) fn new_task_in_scope(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.active_project().map(|p| p.uuid);
        let project = self
            .right_views
            .tasks
            .state
            .scope
            .project(active)
            .or(active);
        if let Some(project) = project.filter(|id| self.project_by_uuid(*id).is_some()) {
            self.open_task_draft_for_project(project, window, cx);
        }
    }

    pub(in crate::workspace) fn clear_task_filters(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.right_views.tasks.state.filter = daruda_store::tasks::TaskFilter::All;
        self.clear_task_search(window, cx);
    }

    /// Clear the Tasks tab search input (the in-field `✕` overlay).
    /// Extracted so the View closure can dispatch in one line.
    pub(super) fn clear_task_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.right_views.tasks.search.clone();
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
        if self.right_views.tasks.state.filter != filter {
            self.right_views.tasks.state.filter = filter;
            cx.notify();
        }
    }

    /// Ensure the Tasks tab's live tick is alive exactly when at least
    /// one task is in the `Running` state. The tick re-renders
    /// the workspace at [`super::task_workflow_ops::TASK_LIVE_TICK`]
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
        if cx.global::<GlobalTasks>().any_active() {
            if self.pumps.task_live_tick.is_none() {
                self.pumps.task_live_tick =
                    Some(crate::workspace::right_dock::task_workflow_ops::spawn_task_live_tick(cx));
            }
        } else {
            self.pumps.task_live_tick = None;
        }
    }

    // ------------------------------------------------------------------
    // CRUD
    // ------------------------------------------------------------------

    /// Update editable fields on an existing task. The lifecycle state
    /// (`Backlog`, `Running`, …) is intentionally *not* part of the
    /// editable surface — that is driven by the workflow.
    ///
    /// Where the task runs is only consulted when a Backlog task starts, so
    /// the machine applies `edit.branch` / `run_in` / `project` to a Backlog
    /// task alone; once started, the branch names the lane it created.
    pub(in crate::workspace) fn update_task(
        &mut self,
        task_id: &str,
        edit: TaskEdit,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<u64, crate::runs::TaskCommitError>> {
        let intent = TaskIntent::Edit {
            id: task_id.to_owned(),
            edit,
        };
        self.propose_task_command(intent, cx)
    }

    // ------------------------------------------------------------------
    // Subtask CRUD
    // ------------------------------------------------------------------
    //
    // Subtask toggles are explicit user commits (checkbox click / Enter /
    // X), not buffered form edits, so each proposes at once and the Task
    // editor's `saved_snapshot` dirty comparison deliberately ignores
    // subtasks.

    /// Append a manually-added subtask to `task_id`. Empty / whitespace
    /// titles are dropped silently — the inline `[+ Add subtask…]` input
    /// drops empty submits without surfacing an error.
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
        let intent = TaskIntent::AddSubtask {
            id: task_id.to_owned(),
            subtask: SubTask::new(trimmed.to_owned()),
        };
        self.propose_task_command(intent, cx).detach();
    }

    /// Flip a subtask's `completed` flag; its `source_session_id` stays, so
    /// an "auto" item the user checks off stays labelled "auto".
    pub(in crate::workspace) fn toggle_subtask(
        &mut self,
        task_id: &str,
        subtask_id: &str,
        cx: &mut Context<Self>,
    ) {
        let intent = TaskIntent::ToggleSubtask {
            id: task_id.to_owned(),
            subtask_id: subtask_id.to_owned(),
        };
        self.propose_task_command(intent, cx).detach();
    }

    /// Remove a subtask. Deletion is immediate with no undo — the X button is
    /// the only path.
    pub(in crate::workspace) fn delete_subtask(
        &mut self,
        task_id: &str,
        subtask_id: &str,
        cx: &mut Context<Self>,
    ) {
        let intent = TaskIntent::DeleteSubtask {
            id: task_id.to_owned(),
            subtask_id: subtask_id.to_owned(),
        };
        self.propose_task_command(intent, cx).detach();
    }

    /// Replace a subtask's title. Empty / whitespace titles are dropped (the
    /// inline rename commits via Enter or blur; destructive removal goes
    /// through the X button), as is a title that did not change.
    pub(in crate::workspace) fn rename_subtask(
        &mut self,
        task_id: &str,
        subtask_id: &str,
        new_title: String,
        cx: &mut Context<Self>,
    ) {
        let trimmed = new_title.trim();
        let unchanged = cx
            .global::<GlobalTasks>()
            .get(task_id)
            .and_then(|task| task.subtasks.iter().find(|s| s.id == subtask_id))
            .is_some_and(|sub| sub.title == trimmed);
        if trimmed.is_empty() || unchanged {
            return;
        }
        let intent = TaskIntent::RenameSubtask {
            id: task_id.to_owned(),
            subtask_id: subtask_id.to_owned(),
            title: trimmed.to_owned(),
        };
        self.propose_task_command(intent, cx).detach();
    }

    /// Permanent delete. The associated lane (if any) is *not* removed — the
    /// user takes care of that from the left dock so two destructive actions
    /// never share one click. (D-1) A run still under way is cancelled first
    /// (the log never deletes an active task), and an editor showing the task
    /// closes once it is gone: it has nothing left to save into.
    pub(in crate::workspace) fn delete_task(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = cx.global::<GlobalTasks>().get(task_id) else {
            return;
        };
        // Drop pending failure tallies before the task itself disappears,
        // otherwise the entries stay forever.
        let stale_sessions = task.session_ids.clone();
        let active = (task.lifecycle.kind() != LifecycleKind::Backlog
            && !task.lifecycle.is_terminal())
        .then(|| task.current_exec().cloned())
        .flatten();
        self.claude_sessions
            .forget_tool_use_failures(&stale_sessions);
        let cancelled = active.map(|exec| self.cancel_execution(exec, cx));
        let id = task_id.to_owned();
        cx.spawn_in(window, async move |ws, cx| {
            if let Some(cancelled) = cancelled
                && cancelled.await.is_err()
            {
                return;
            }
            let Ok(deleted) = ws.update(cx, |ws, cx| {
                let intent = TaskIntent::Delete { id: id.clone() };
                ws.propose_task_command(intent, cx)
            }) else {
                return;
            };
            if deleted.await.is_err() {
                return;
            }
            // SILENT-OK: the window closed; its editor went with it.
            let _ = ws.update_in(cx, |ws, window, cx| {
                ws.close_deleted_task_editor(&id, window, cx)
            });
        })
        .detach();
    }

    /// Close the editor showing a task that no longer exists.
    fn close_deleted_task_editor(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(detail) = &self.pages.tasks.detail
            && detail.editor.task_id.as_deref() == Some(task_id)
        {
            let id = crate::workspace::pages::detail::PageDetailId::TaskEditor(detail.id);
            self.drop_gone_detail(id, window, cx);
        }
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
        let items = TaskPickerModal::build_items(cx.global::<GlobalTasks>().book(), action);
        let weak = cx.weak_entity();
        crate::workspace::dialog_helpers::open_form_modal(
            action.modal_title(),
            None,
            move |window, cx| TaskPickerModal::new(weak.clone(), action, items.clone(), window, cx),
            window,
            cx,
        );
    }

    /// Open an OK-only alert dialog showing why `task_id` failed. No-op
    /// for tasks with no failure to show — the menu entry is only exposed
    /// on failed rows, but defensive guarding keeps the dispatcher honest
    /// if a race lets the state flip between menu open and click.
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
            match super::tasks::failure_text(&task.lifecycle) {
                Some(message) => message,
                None => return,
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
            crate::surface::strings::task::confirm_delete_body(&task.spec.title)
        };
        let weak = cx.weak_entity();
        let id = task_id.to_string();
        crate::ui::dialog::open_confirm_dialog(
            crate::surface::strings::task::picker_title_delete(),
            body,
            crate::surface::strings::common::btn_delete(),
            ButtonVariant::Danger,
            move |_, window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    let id = id.clone();
                    ws.update(app_cx, |ws, cx| ws.delete_task(&id, window, cx));
                }
            },
            window,
            cx,
        );
    }
}
