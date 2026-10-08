//! Saving the Task editor's form: read it off the editor, check where the task
//! runs, and persist it into `GlobalTasks`.

use daruda_store::tasks::{TaskId, TaskRunIn};
use gpui::{BorrowAppContext as _, Context, Window};

use super::TaskEditorId;
use super::state::{RunInChoice, TaskEditValues};
use crate::workspace::Workspace;

impl Workspace {
    /// Commit the form as shown, including a typed-but-unsubmitted subtask
    /// and an open rename — the sequence every interactive save shares. An
    /// invalid form commits nothing, so the editor it stays in is unchanged.
    pub(in crate::workspace) fn commit_task_editor(
        &mut self,
        editor_id: TaskEditorId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TaskId> {
        if !self.task_editor(editor_id)?.can_save(cx) {
            return None;
        }
        self.submit_new_subtask(editor_id, window, cx);
        self.commit_rename_subtask(editor_id, cx);
        self.commit_task_form(editor_id, cx)
    }

    /// Persist the form into `GlobalTasks`, leaving the editor open.
    /// Returns the resolved `task_id`, or `None` when the form is
    /// invalid — the caller then keeps the editor.
    pub(in crate::workspace) fn commit_task_form(
        &mut self,
        editor_id: TaskEditorId,
        cx: &mut Context<Self>,
    ) -> Option<TaskId> {
        let form = self.read_task_edit_form(editor_id, cx)?;
        let values = &form.values;
        if values.title.trim().is_empty() {
            return None;
        }
        // `None` once started: where the task runs is then fixed.
        let run_in = if form.editable {
            Some(self.commit_task_run_in(editor_id, values, form.project)?)
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
                        project: form.project.filter(|_| form.editable),
                    },
                    cx,
                );
                id.clone()
            }
            None => {
                // A new task belongs to the project the form names; with none
                // open, it has nowhere to run.
                let project = form.project?;
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

        // Re-baseline the dirty snapshot so the form no longer reads
        // as dirty after a successful save.
        if let Some(te) = self.task_editor_mut(editor_id) {
            te.task_id = Some(task_id.clone());
            te.draft_subtasks.clear();
            te.saved_snapshot = te.current_snapshot(cx);
        }

        Some(task_id)
    }

    /// The location a still-editable form commits to, or `None` when it is
    /// not savable. The branch is checked again here because a lane created
    /// after the form opened can have taken it since the last keystroke.
    fn commit_task_run_in(
        &mut self,
        editor_id: TaskEditorId,
        form: &TaskEditValues,
        project: Option<daruda_store::project::ProjectUuid>,
    ) -> Option<TaskRunIn> {
        match form.run_in {
            RunInChoice::NewWorktree => {
                let validation = self.branch_validation_for(&form.branch, true, project);
                let invalid = validation.is_invalid();
                if let Some(te) = self.task_editor_mut(editor_id) {
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

    /// Read the current form values in one step, so no borrow of the
    /// editor outlives the snapshot.
    fn read_task_edit_form(
        &self,
        editor_id: TaskEditorId,
        cx: &Context<Self>,
    ) -> Option<TaskEditForm> {
        let te = self.task_editor(editor_id)?;
        Some(TaskEditForm {
            task_id: te.task_id.clone(),
            editable: super::run_in_ops::location_editable(te, cx.global()),
            project: te.project(cx),
            values: te.current_snapshot(cx),
        })
    }
}

/// What Save reads off the editor in one step, so it holds no borrow of
/// the editor past the read.
struct TaskEditForm {
    task_id: Option<TaskId>,
    /// Whether the task has yet to start, so its location may still change.
    editable: bool,
    project: Option<daruda_store::project::ProjectUuid>,
    values: TaskEditValues,
}
