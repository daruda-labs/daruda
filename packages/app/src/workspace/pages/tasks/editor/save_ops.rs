//! Saving the Task editor's form: read it off the editor, check where the task
//! runs, and propose it to the task log.

use crate::runs::TaskIntent;
use chrono::Utc;
use daruda_store::tasks::machine::TaskEdit;
use daruda_store::tasks::{TaskId, TaskRunIn, TaskSpec};
use futures::FutureExt as _;
use gpui::{Context, Window};

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
    ) -> Option<(TaskId, gpui::Task<bool>)> {
        if !self.task_editor(editor_id)?.can_save(cx) {
            return None;
        }
        self.submit_new_subtask(editor_id, window, cx);
        self.commit_rename_subtask(editor_id, cx);
        self.begin_task_form_save(editor_id, cx)
    }

    /// Propose the form to the task log, leaving the editor open. Returns
    /// the resolved `task_id`, or `None` when the form is invalid — the
    /// caller then keeps the editor. The task shows in the projection once
    /// the log commits it; `RunsHost::barrier` waits for that.
    #[cfg(test)]
    pub(in crate::workspace) fn commit_task_form(
        &mut self,
        editor_id: TaskEditorId,
        cx: &mut Context<Self>,
    ) -> Option<TaskId> {
        let (id, saved) = self.begin_task_form_save(editor_id, cx)?;
        saved.detach();
        Some(id)
    }

    fn begin_task_form_save(
        &mut self,
        editor_id: TaskEditorId,
        cx: &mut Context<Self>,
    ) -> Option<(TaskId, gpui::Task<bool>)> {
        if self.task_editor(editor_id)?.pending_save.is_some() {
            return None;
        }
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

        let mut commits = Vec::new();
        let task_id = match &form.task_id {
            Some(id) => {
                cx.global::<crate::runs::GlobalTasks>().get(id)?;
                let edit = TaskEdit {
                    title: values.title.clone(),
                    prompt: values.prompt.clone(),
                    notes: values.notes.clone(),
                    auto_execute: values.auto_execute,
                    agent_surface: values.agent_surface,
                    base_worktree_path: base_path.clone(),
                    branch,
                    run_in,
                    project: form.project.filter(|_| form.editable),
                };
                commits.push(self.update_task(id, edit, cx));
                id.clone()
            }
            None => {
                // A new task belongs to the project the form names; with none
                // open, it has nowhere to run.
                let project = form.project?;
                let now = Utc::now();
                let mut spec = TaskSpec::new(
                    project,
                    values.title.clone(),
                    values.prompt.clone(),
                    base_path.clone(),
                    now,
                );
                if let Some(branch) = branch {
                    spec.branch_name = branch;
                }
                spec.run_in = run_in.unwrap_or_default();
                spec.notes = values.notes.clone();
                spec.auto_execute = values.auto_execute;
                spec.agent_surface = values.agent_surface;
                let new_id = spec.id.clone();
                // One log, one queue: the subtasks follow their task in order.
                commits.push(self.propose_task_command(TaskIntent::Create { spec }, cx));
                for subtask in values.draft_subtasks.iter().cloned() {
                    let intent = TaskIntent::AddSubtask {
                        id: new_id.clone(),
                        subtask,
                    };
                    commits.push(self.propose_task_command(intent, cx));
                }
                new_id
            }
        };

        self.set_task_editor_pending_save(editor_id, Some(task_id.clone()), cx);
        let mut committed = Box::pin(futures::future::join_all(commits));
        if let Some(results) = committed.as_mut().now_or_never() {
            let success = results.iter().all(Result::is_ok);
            let saved =
                self.finish_task_form_save(editor_id, &task_id, values.clone(), success, cx);
            return success.then(|| (task_id, gpui::Task::ready(saved)));
        }
        let saved_id = task_id.clone();
        let values = values.clone();
        let (answer, answered) = smol::channel::bounded(1);
        cx.spawn(async move |workspace, cx| {
            let success = committed.await.iter().all(Result::is_ok);
            let applied = workspace.update(cx, |workspace, cx| {
                workspace.finish_task_form_save(editor_id, &saved_id, values, success, cx)
            });
            let saved = match applied {
                Ok(saved) => saved,
                Err(error) => {
                    daruda_store::observability::log_writer::LogWriter::log(
                        daruda_store::observability::error_report::ErrorReport::new(
                            "Task save could not reach its editor",
                        )
                        .message(error.to_string())
                        .at(file!(), line!())
                        .dedup("tasks.editor.save")
                        .build(),
                    );
                    false
                }
            };
            // SILENT-OK: the save's caller stopped listening.
            let _ = answer.try_send(saved);
        })
        .detach();
        Some((
            task_id,
            cx.spawn(async move |_, _| answered.recv().await.unwrap_or(false)),
        ))
    }

    fn finish_task_form_save(
        &mut self,
        editor_id: TaskEditorId,
        task_id: &str,
        mut values: TaskEditValues,
        success: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        self.set_task_editor_pending_save(editor_id, None, cx);
        if let Some(editor) = self.task_editor_mut(editor_id)
            && success
        {
            editor.task_id = Some(task_id.to_owned());
            editor.draft_subtasks.clear();
            values.draft_subtasks.clear();
            editor.saved_snapshot = values;
        }
        cx.notify();
        success
            && self
                .task_editor(editor_id)
                .is_some_and(|editor| !editor.is_dirty(cx))
    }

    pub(super) fn set_task_editor_pending_save(
        &mut self,
        editor_id: TaskEditorId,
        task_id: Option<TaskId>,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.task_editor_mut(editor_id) else {
            return;
        };
        editor.pending_save = task_id;
        let disabled = !editor.checklist_editable();
        let inputs = [
            editor.new_subtask_input.clone(),
            editor.editing_subtask_input.clone(),
        ];
        for input in inputs {
            input.update(cx, |input, cx| input.set_disabled(disabled, cx));
        }
        cx.notify();
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
