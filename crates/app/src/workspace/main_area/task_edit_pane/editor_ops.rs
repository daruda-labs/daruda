//! In-pane authoring actions and draft checklist edits.

use super::super::pane_tree::PaneId;
use crate::workspace::Workspace;
#[cfg(feature = "screenshot")]
use daruda_store::tasks::Task;
use daruda_store::tasks::{SubTask, TaskAgentSurface};
#[cfg(feature = "screenshot")]
use gpui::BorrowAppContext as _;
use gpui::{Context, Focusable as _, Window};

impl Workspace {
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn seed_task_editor_for_shot(
        &mut self,
        preview: bool,
        running: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_lane().is_none() {
            self.add_project(self.data_dir.clone(), window, cx);
        }
        let Some(project) = self.active_project().map(|p| p.uuid) else {
            return;
        };
        let mut task = Task::new(
            project,
            "Improve task creation and editing".into(),
            "## Goal\nMake task authoring feel clear and predictable.\n\n- Keep the prompt in focus\n- Preserve drafts when navigating\n- Verify save and start independently\n\n**Acceptance:** keyboard navigation and both themes work.".into(),
            None,
        );
        task.subtasks = vec![
            SubTask::new("Compare Orca and Superset task flows".into()),
            SubTask::new("Verify narrow panes and keyboard navigation".into()),
        ];
        task.subtasks[0].completed = true;
        let Some(lane_path) = self.active_lane().map(|lane| lane.path.clone()) else {
            return;
        };
        let task_id = if running {
            task.execution = Some(daruda_store::tasks::TaskExecution {
                source: Default::default(),
                id: "screenshot-task-run".into(),
                agent_id: self.agents[0].id.clone(),
                account_id: None,
                cwd: lane_path.clone(),
                session_id: Some("screenshot-task-session".into()),
            });
            task.state = daruda_store::tasks::TaskState::Running {
                worktree_path: lane_path,
            };
            let id = task.id.clone();
            cx.update_global::<crate::agent::tasks_global::GlobalTasks, _>(|g, _| {
                g.add(task.clone());
            });
            Some(id)
        } else {
            None
        };
        self.open_task_edit_pane(task_id, window, cx);
        let pane_id = self.active_runtime().focused_pane_id;
        let te = self.task_edit_content_for_pane(pane_id).unwrap();
        let title = te.title_input.clone();
        let prompt = te.prompt_state.clone();
        title.update(cx, |s, cx| s.set_value(task.title, window, cx));
        prompt.update(cx, |s, cx| s.set_value(task.prompt, window, cx));
        self.refresh_task_edit_title(pane_id, cx);
        if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.preview_prompt = preview;
            te.settings_open = preview || running;
            if !running {
                te.draft_subtasks = task.subtasks;
            }
        }
        cx.notify();
    }

    /// Add a checklist item to the local draft or its persisted task.
    pub(super) fn submit_new_subtask(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.active_runtime().panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(te) = pane.task_edit_content() else {
            return;
        };
        let task_id = te.task_id.clone();
        let text = te.new_subtask_input.read(cx).value().to_string();
        let input = te.new_subtask_input.clone();
        if text.trim().is_empty() {
            return;
        }
        if let Some(task_id) = task_id {
            self.add_subtask(&task_id, text, cx);
        } else if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.draft_subtasks
                .push(SubTask::new(text.trim().to_string()));
        }
        cx.notify();
        input.update(cx, |inp, cx_state| {
            inp.set_value(String::new(), window, cx_state)
        });
    }

    /// Begin an inline rename of `subtask_id`. Stamps the shared
    /// rename input with the current title and routes focus to it so
    /// the user can edit immediately. Only one rename can be active at
    /// a time (single shared input).
    pub(super) fn enter_rename_subtask(
        &mut self,
        pane_id: PaneId,
        subtask_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.active_runtime().panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(te) = pane.task_edit_content() else {
            return;
        };
        let input_entity = te.editing_subtask_input.clone();
        let subtasks = te
            .task_id
            .as_deref()
            .and_then(|id| {
                cx.global::<crate::agent::tasks_global::GlobalTasks>()
                    .get(id)
            })
            .map(|task| &task.subtasks)
            .unwrap_or(&te.draft_subtasks);
        let title = subtasks
            .iter()
            .find(|s| s.id == subtask_id)
            .map(|s| s.title.clone())
            .unwrap_or_default();
        input_entity.update(cx, |inp, cx_state| inp.set_value(title, window, cx_state));
        if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.editing_subtask = Some(subtask_id);
        }
        let handle = input_entity.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    pub(super) fn handle_task_subtask_mouse_down(
        &mut self,
        pane_id: PaneId,
        subtask_id: &str,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.click_count >= 2 {
            self.enter_rename_subtask(pane_id, subtask_id.to_string(), window, cx);
            cx.stop_propagation();
        }
    }

    /// Commit the inline rename — flushes the input's text into
    /// `rename_subtask` and clears the editing state. Empty / unchanged
    /// titles are dropped by `rename_subtask` itself.
    pub(super) fn commit_rename_subtask(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let Some(pane) = self.active_runtime().panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(te) = pane.task_edit_content() else {
            return;
        };
        let task_id = te.task_id.clone();
        let Some(subtask_id) = te.editing_subtask.clone() else {
            return;
        };
        let new_title = te.editing_subtask_input.read(cx).text().to_string();
        if let Some(task_id) = task_id {
            self.rename_subtask(&task_id, &subtask_id, new_title, cx);
        } else if let Some(te) = self.task_edit_content_mut_for_pane(pane_id)
            && let Some(subtask) = te.draft_subtasks.iter_mut().find(|s| s.id == subtask_id)
            && !new_title.trim().is_empty()
        {
            subtask.title = new_title.trim().to_string();
        }
        if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.editing_subtask = None;
        }
        cx.notify();
    }

    /// Cancel the inline rename without touching the underlying
    /// subtask. Reached from the TaskEdit pane's outer Esc handler —
    /// `gpui_component::Input` doesn't emit a Cancel event of its own,
    /// so Escape routing lives one level up in `task_edit_pane::render`.
    pub(super) fn cancel_rename_subtask(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.editing_subtask = None;
        }
        cx.notify();
    }

    pub(super) fn save_task_editor(
        &mut self,
        pane_id: PaneId,
        start: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.submit_new_subtask(pane_id, window, cx);
        self.commit_rename_subtask(pane_id, cx);
        let Some(id) = self.commit_task_edit_pane(pane_id, cx) else {
            return;
        };
        let branch = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .get(&id)
            .map(|task| task.branch_name.clone());
        if let Some(branch) = branch
            && let Some(te) = self.task_edit_content_for_pane(pane_id)
        {
            let input = te.branch_input.clone();
            if input.read(cx).value().as_ref() != branch {
                input.update(cx, |state, cx| state.set_value(branch.clone(), window, cx));
            }
            if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
                te.saved_snapshot.branch = branch;
                te.branch_validation =
                    super::task_edit_ops::validate_branch(&te.saved_snapshot.branch);
            }
        }
        if start {
            self.start_task(&id, window, cx);
        }
        cx.notify();
    }

    pub(super) fn toggle_task_settings(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.settings_open = !te.settings_open;
        }
        cx.notify();
    }

    pub(super) fn toggle_task_notes(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.notes_open = !te.notes_open;
        }
        cx.notify();
    }

    pub(super) fn set_task_prompt_preview(
        &mut self,
        pane_id: PaneId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.preview_prompt = index == 1;
        }
        cx.notify();
    }

    pub(super) fn set_task_surface(
        &mut self,
        pane_id: PaneId,
        surface: TaskAgentSurface,
        cx: &mut Context<Self>,
    ) {
        if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.agent_surface = surface;
        }
        cx.notify();
    }

    pub(super) fn set_task_auto_execute(
        &mut self,
        pane_id: PaneId,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.auto_execute = enabled;
        }
        cx.notify();
    }

    /// Replace the branch with a fresh `task-<random>` name.
    pub(super) fn regenerate_task_branch(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(te) = self.task_edit_content_for_pane(pane_id) else {
            return;
        };
        let branch = daruda_store::tasks::random_branch_name();
        let branch_input = te.branch_input.clone();
        let validation = self.branch_validation_for(&branch, true);
        if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.branch_validation = validation;
        }
        branch_input.update(cx, |input, cx| input.set_value(branch, window, cx));
        cx.notify();
    }

    pub(super) fn toggle_editor_subtask(
        &mut self,
        pane_id: PaneId,
        subtask_id: &str,
        cx: &mut Context<Self>,
    ) {
        let task_id = self
            .task_edit_content_for_pane(pane_id)
            .and_then(|te| te.task_id.clone());
        if let Some(id) = task_id {
            self.toggle_subtask(&id, subtask_id, cx);
        } else if let Some(te) = self.task_edit_content_mut_for_pane(pane_id)
            && let Some(subtask) = te.draft_subtasks.iter_mut().find(|s| s.id == subtask_id)
        {
            subtask.completed = !subtask.completed;
        }
        cx.notify();
    }

    pub(super) fn delete_editor_subtask(
        &mut self,
        pane_id: PaneId,
        subtask_id: &str,
        cx: &mut Context<Self>,
    ) {
        let task_id = self
            .task_edit_content_for_pane(pane_id)
            .and_then(|te| te.task_id.clone());
        if let Some(id) = task_id {
            self.delete_subtask(&id, subtask_id, cx);
        } else if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.draft_subtasks.retain(|s| s.id != subtask_id);
        }
        if let Some(te) = self.task_edit_content_mut_for_pane(pane_id)
            && te.editing_subtask.as_deref() == Some(subtask_id)
        {
            te.editing_subtask = None;
        }
        cx.notify();
    }

    pub(super) fn handle_task_edit_key(
        &mut self,
        pane_id: PaneId,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape"
            && self
                .task_edit_content_for_pane(pane_id)
                .is_some_and(|te| te.editing_subtask.is_some())
        {
            self.cancel_rename_subtask(pane_id, cx);
            cx.stop_propagation();
        }
    }

    pub(super) fn open_editor_task_worktree(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self
            .task_edit_content_for_pane(pane_id)
            .and_then(|te| te.task_id.clone())
        {
            self.focus_task_lane(&id, window, cx);
        }
    }

    pub(super) fn open_editor_prompt_file(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self
            .task_edit_content_for_pane(pane_id)
            .and_then(|te| te.task_id.clone())
        {
            self.open_task_prompt_file(&id, window, cx);
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
