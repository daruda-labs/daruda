//! Saving the TaskEdit form: read it off the pane, check where the task
//! runs, and persist it into `GlobalTasks`.

use daruda_store::tasks::{TaskId, TaskRunIn};
use gpui::{BorrowAppContext as _, Context, Window};

use super::state::{RunInChoice, TaskEditValues};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

impl Workspace {
    /// Persist the TaskEdit pane (`pane_id`) into `GlobalTasks`. When
    /// `task_id = None` this creates a new task; otherwise it updates
    /// the existing one. When `start = true` the task transitions to
    /// `Running` immediately via `start_task`. The pane closes on
    /// success.
    pub(in crate::workspace) fn save_task_edit_pane(
        &mut self,
        pane_id: PaneId,
        start: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task_id) = self.commit_task_edit_pane(pane_id, cx) else {
            return;
        };

        self.close_pane_by_id(pane_id, window, cx);

        if start {
            self.start_task(&task_id, window, cx);
        }
        cx.notify();
    }

    /// Persist the pane's form into `GlobalTasks` without closing the
    /// pane. Used by the close-tab and window-close batch flows
    /// where one wrapping prompt covers multiple panes and the
    /// caller drives the close pass separately. Returns the resolved
    /// `task_id` on success, `None` when the form is invalid (the
    /// caller should keep the pane open in that case).
    pub(in crate::workspace) fn commit_task_edit_pane(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> Option<TaskId> {
        let form = self.read_task_edit_form(pane_id, cx)?;
        let values = &form.values;
        if values.title.trim().is_empty() {
            return None;
        }
        // `None` once started: where the task runs is then fixed.
        let run_in = if form.editable {
            Some(self.commit_task_run_in(pane_id, values)?)
        } else {
            None
        };

        // Empty sentinel → `None`; non-empty → registered lane
        // path. The path is round-tripped as a string through the
        // `SelectState` value, which means we can't statically
        // distinguish "not in the lane list anymore" from "user
        // picked a stale option" — but `start_task` re-runs
        // `branch_for_worktree_path` and falls back to git's default
        // when the lookup misses, so the worst case is the same
        // behaviour as `None`.
        // A base is only branched from when a worktree is created.
        let base_path: Option<std::path::PathBuf> = if values.base_value.is_empty()
            || matches!(run_in, Some(TaskRunIn::ExistingLane { .. }))
        {
            None
        } else {
            Some(std::path::PathBuf::from(&values.base_value))
        };

        // The rule walk trims, so the value it accepted is the one stored.
        let branch = daruda_core::git::validate_branch_name(&values.branch)
            .ok()
            .map(str::to_owned);

        let task_id = match &form.task_id {
            Some(id) => {
                cx.global::<crate::agent::tasks_global::GlobalTasks>()
                    .get(id)?;
                self.update_task(
                    id,
                    crate::workspace::right_dock::task_ops::TaskEdits {
                        title: values.title.clone(),
                        prompt: values.prompt.clone(),
                        notes: values.notes.clone(),
                        auto_execute: values.auto_execute,
                        agent_surface: values.agent_surface,
                        base_worktree_path: base_path.clone(),
                        branch,
                        run_in,
                    },
                    cx,
                );
                id.clone()
            }
            None => {
                // A task belongs to the project it was written in.
                let project = self.active_project()?.uuid;
                let mut task = daruda_store::tasks::Task::new(
                    project,
                    values.title.clone(),
                    values.prompt.clone(),
                    base_path.clone(),
                );
                if let Some(branch) = branch {
                    task.branch_name = branch;
                }
                task.run_in = run_in.unwrap_or_default();
                task.subtasks = values.draft_subtasks.clone();
                task.notes = values.notes.clone();
                task.auto_execute = values.auto_execute;
                task.agent_surface = values.agent_surface;
                let new_id = task.id.clone();
                cx.update_global::<crate::agent::tasks_global::GlobalTasks, _>(|g, _| {
                    g.add(task);
                });
                self.save_tasks_dirty(cx);
                new_id
            }
        };

        // Re-baseline the dirty snapshot so the pane no longer reads
        // as dirty after a successful save.
        if let Some(te) = self.task_edit_content_mut_for(pane_id) {
            te.task_id = Some(task_id.clone());
            te.draft_subtasks.clear();
            te.saved_snapshot = te.current_snapshot(cx);
        }

        Some(task_id)
    }

    /// The location a still-editable form commits to, or `None` when it is
    /// not savable. The branch is checked again here because a lane created
    /// after the form opened can have taken it since the last keystroke.
    fn commit_task_run_in(&mut self, pane_id: PaneId, form: &TaskEditValues) -> Option<TaskRunIn> {
        match form.run_in {
            RunInChoice::NewWorktree => {
                let validation = self.branch_validation_for(&form.branch, true);
                let invalid = validation.is_invalid();
                if let Some(te) = self.task_edit_content_mut_for(pane_id) {
                    te.branch_validation = validation;
                }
                (!invalid).then_some(TaskRunIn::NewWorktree)
            }
            RunInChoice::ExistingLane => {
                (!form.lane_value.is_empty()).then(|| TaskRunIn::ExistingLane {
                    path: std::path::PathBuf::from(&form.lane_value),
                })
            }
        }
    }

    /// Read the current form values without holding a `&mut self`
    /// borrow on `self.active_runtime().panes` past the snapshot.
    fn read_task_edit_form(&self, pane_id: PaneId, cx: &Context<Self>) -> Option<TaskEditForm> {
        let te = self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .find(|p| p.id == pane_id)?
            .task_edit_content()?;
        Some(TaskEditForm {
            task_id: te.task_id.clone(),
            editable: super::run_in_ops::location_editable(te, cx.global()),
            values: te.current_snapshot(cx),
        })
    }
}

/// What Save reads off the pane in one step, so it holds no borrow of
/// `self.active_runtime().panes` past the read.
struct TaskEditForm {
    task_id: Option<TaskId>,
    /// Whether the task has yet to start, so its location may still change.
    editable: bool,
    values: TaskEditValues,
}
