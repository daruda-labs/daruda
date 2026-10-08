//! The Task editor's lifecycle — builder, open, branch validation, and the
//! accessors the sibling `save_ops` / `prompt_file_ops` reach the editor by.
//!
//! The editor itself is rendered by `editor::render_detail`; this
//! module owns the *operations* (open, restore, validate)
//! that the renderer + status_pill + Tasks-tab row click dispatch into.
//!
//! Branch validation runs the shared `daruda_core::git` rule walk and
//! renders the rule it broke, so the form's inline diagnostic and the
//! silent filter behind `sanitize_branch_name` can never disagree on what
//! git accepts.

use daruda_store::project::{ProjectUuid, TaskDetailTarget};
use daruda_store::tasks::{Task, TaskAgentSurface, TaskId, random_branch_name};
use gpui::{AppContext as _, Context, Focusable as _, SharedString, Window};

use super::TaskEditorId;
use super::prompt_file_ops::install_prompt_watcher;
use super::state::{BranchValidation, TaskEditContent, TaskEditValues};
use crate::ui::select::{SelectOption, state_with_options};
use crate::ui::{InputEvent, InputState, make_markdown_prose_state};
use crate::workspace::Workspace;
use crate::workspace::pages::tasks::TaskDetail;

/// Validate a branch-input string, reporting which rule it broke so the
/// form can show a precise red label. An empty field is not an error — a
/// draft then gets its default `task-<id>` branch, a saved task keeps its own.
pub(super) fn validate_branch(text: &str) -> BranchValidation {
    match daruda_core::git::validate_branch_name(text) {
        Ok(_) => BranchValidation::Valid,
        Err(daruda_core::git::BranchNameRule::Empty) => BranchValidation::Empty,
        Err(rule) => BranchValidation::Invalid {
            reason: SharedString::from(crate::surface::strings::branch_rule::reason(rule)),
        },
    }
}

impl Workspace {
    /// Open the editor for `task_id` (`None`: a fresh draft) as the Tasks
    /// page's detail — see [`Self::open_task_editor_with`].
    pub(in crate::workspace) fn open_task_editor(
        &mut self,
        task_id: Option<TaskId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_task_editor_with(task_id, None, window, cx);
    }

    pub(in crate::workspace) fn open_task_draft_for_project(
        &mut self,
        project: ProjectUuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_task_editor_with(None, Some(project), window, cx);
    }

    /// Show `task_id`'s editor (`None`: a fresh draft) as the Tasks page's
    /// detail. The same task already open is shown again as it is; anything
    /// else replaces the current editor, asking first if it holds edits.
    fn open_task_editor_with(
        &mut self,
        task_id: Option<TaskId>,
        draft_project: Option<ProjectUuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(open) = self.pages.tasks.detail.as_ref()
            && task_id.is_some()
            && open.editor.task_id == task_id
        {
            let id = open.id;
            self.show_task_detail(id, window, cx);
            return;
        }
        self.leave_task_detail_then(window, cx, move |ws, window, cx| {
            let initial = task_id.as_deref().and_then(|id| {
                cx.global::<crate::agent::tasks_global::GlobalTasks>()
                    .get(id)
                    .cloned()
            });
            let id = ws.install_task_editor(task_id, initial, draft_project, window, cx);
            ws.show_task_detail(id, window, cx);
        });
    }

    /// Reopen the editor a saved workspace showed, as a fresh form. A task
    /// deleted since, or a draft whose project is no longer open, leaves the
    /// page on its list.
    pub(in crate::workspace) fn restore_task_detail(
        &mut self,
        target: &TaskDetailTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            TaskDetailTarget::Task { id } => {
                let Some(task) = cx
                    .global::<crate::agent::tasks_global::GlobalTasks>()
                    .get(id)
                    .cloned()
                else {
                    return;
                };
                self.install_task_editor(Some(id.clone()), Some(task), None, window, cx);
            }
            TaskDetailTarget::NewDraft { project } => {
                if self.project_by_uuid(*project).is_some() {
                    self.install_task_editor(None, None, Some(*project), window, cx);
                }
            }
        }
    }

    /// Make a new editor the page's detail. The caller has left the old one.
    fn install_task_editor(
        &mut self,
        task_id: Option<TaskId>,
        initial: Option<Task>,
        draft_project: Option<ProjectUuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TaskEditorId {
        let id = TaskEditorId(self.alloc_id());
        let editor = self.build_task_editor(id, task_id, initial, draft_project, window, cx);
        self.mutate_durable(cx, |ws, _| {
            ws.pages.tasks.detail = Some(TaskDetail { id, editor });
        });
        id
    }

    /// Bring the Tasks page up on editor `id` and put the cursor in its title.
    fn show_task_detail(&mut self, id: TaskEditorId, window: &mut Window, cx: &mut Context<Self>) {
        self.show_page(crate::workspace::pages::Page::Tasks, cx);
        if let Some(te) = self.task_editor(id) {
            te.title_input.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    fn build_task_editor(
        &mut self,
        editor_id: TaskEditorId,
        task_id: Option<TaskId>,
        initial: Option<Task>,
        draft_project: Option<ProjectUuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TaskEditContent {
        let (title, prompt, notes, branch_name, auto_execute, agent_surface) = match &initial {
            Some(t) => (
                t.title.clone(),
                t.prompt.clone(),
                t.notes.clone(),
                t.branch_name.clone(),
                t.auto_execute,
                t.agent_surface,
            ),
            // A draft opens with a usable branch already filled in.
            None => (
                String::new(),
                String::new(),
                String::new(),
                random_branch_name(),
                true,
                TaskAgentSurface::default(),
            ),
        };

        let title_for_default = title.clone();
        let title_input = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state)
                .placeholder(crate::surface::strings::task::edit_title_placeholder());
            if !title_for_default.is_empty() {
                s = s.default_value(title_for_default);
            }
            s
        });

        let branch_for_default = branch_name.clone();
        let branch_input = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state)
                .placeholder(crate::surface::strings::task::edit_branch_placeholder());
            if !branch_for_default.is_empty() {
                s = s.default_value(branch_for_default);
            }
            s
        });

        // TaskEdit prompt + notes are markdown prose — the prose
        // factory hides the line-number gutter so the editor reads
        // like a plain textarea. Use `make_markdown_state` instead
        // for code-style buffers (gpui_component default keeps line
        // numbers on).
        let prompt_state = make_markdown_prose_state(
            &prompt,
            crate::surface::strings::task::edit_prompt_placeholder(),
            crate::ui::theme::TASK_EDIT_PROMPT_ROWS,
            window,
            cx,
        );
        let notes_state = make_markdown_prose_state(
            &notes,
            crate::surface::strings::task::edit_notes_placeholder(),
            crate::ui::theme::TASK_EDIT_NOTES_ROWS,
            window,
            cx,
        );
        let [prompt_sub, notes_sub] = [&prompt_state, &notes_state].map(|state| {
            cx.subscribe(state, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            })
        });

        let title_sub = cx.subscribe_in(
            &title_input,
            window,
            move |this, _inp, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.refresh_task_edit_title(editor_id, cx);
                }
            },
        );
        let branch_sub = cx.subscribe_in(
            &branch_input,
            window,
            move |this, _inp, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.on_task_edit_branch_typed(editor_id, cx);
                }
            },
        );

        // Subtask inputs — one for the trailing `[+ Add subtask…]`
        // row, one shared across all inline-rename rows. Both go
        // through `gpui_component::Input` (IME-verified).
        let new_subtask_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(crate::surface::strings::task::subtask_add_placeholder())
        });
        let editing_subtask_input = cx.new(|cx_state| InputState::new(window, cx_state));
        let new_subtask_sub = cx.subscribe_in(
            &new_subtask_input,
            window,
            move |this, _inp, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.submit_new_subtask(editor_id, window, cx);
                }
            },
        );
        // Cancel for inline-rename runs through the modal's outer key
        // handler — `InputEvent` doesn't expose Escape. Enter commits.
        let rename_subtask_sub = cx.subscribe_in(
            &editing_subtask_input,
            window,
            move |this, _inp, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.commit_rename_subtask(editor_id, cx);
                }
            },
        );

        // Base-lane selector. The leading
        // empty-string sentinel maps to `Task::base_worktree_path ==
        // None`; the remaining options are registered lanes keyed
        // by absolute path. Building the option list once here keeps
        // the dropdown stable across rerenders — re-deriving on every
        // frame would burn allocations and reset list-search state.
        // The task's own project when it is open here, else the active one.
        let project = initial
            .as_ref()
            .map(|t| t.project)
            .or(draft_project)
            .filter(|uuid| self.project_by_uuid(*uuid).is_some())
            .or_else(|| self.active_project().map(|p| p.uuid));
        let project_select = cx.new(|cx| {
            state_with_options(
                project_options(self),
                project.map(super::state::project_value).as_ref(),
                window,
                cx,
            )
        });
        let project_sub = cx.subscribe_in(
            &project_select,
            window,
            move |this, _state, ev: &crate::ui::select::ConfirmEvent, window, cx| {
                if matches!(ev, crate::ui::select::SelectEvent::Confirm(_)) {
                    this.on_task_edit_project_changed(editor_id, window, cx);
                }
            },
        );

        let base_options = base_lane_options(self, project);
        let base_initial: Option<SharedString> = initial
            .as_ref()
            .and_then(|t| t.base_worktree_path.as_ref())
            .and_then(|p| p.to_str())
            .map(|s| SharedString::from(s.to_string()));
        let base_select =
            cx.new(|cx| state_with_options(base_options, base_initial.as_ref(), window, cx));
        // `Confirm` fires once the dropdown commits the user's pick.
        // The actual value lives on `SelectState`; this listener only
        // exists to invalidate the dirty-comparison snapshot on the
        // next render, the same way `branch_input::Changed` does.
        let base_sub = cx.subscribe_in(
            &base_select,
            window,
            move |_this, _state, ev: &crate::ui::select::ConfirmEvent, _window, cx| {
                if matches!(ev, crate::ui::select::SelectEvent::Confirm(_)) {
                    cx.notify();
                }
            },
        );

        let focus_handle = cx.focus_handle();

        let cached_title: SharedString = if title.is_empty() {
            crate::surface::strings::command::new_task().into()
        } else {
            SharedString::from(title.clone())
        };

        let editable = super::run_in_ops::not_started(initial.as_ref());
        let branch_validation = self.branch_validation_for(&branch_name, editable, project);
        let run_in = super::run_in_ops::run_in_choice(initial.as_ref());
        let lane_initial = super::run_in_ops::initial_lane(initial.as_ref(), self, project);
        let lane_select = cx.new(|cx| {
            state_with_options(
                super::run_in_ops::lane_options(self, project),
                lane_initial.as_ref(),
                window,
                cx,
            )
        });
        let lane_sub = cx.subscribe_in(
            &lane_select,
            window,
            move |_this, _state, ev: &crate::ui::select::ConfirmEvent, _window, cx| {
                if matches!(ev, crate::ui::select::SelectEvent::Confirm(_)) {
                    cx.notify();
                }
            },
        );

        // Running tasks watch immediately; Start attaches the watcher once
        // it has materialized the task's prompt file.
        let (_prompt_watcher, _prompt_pump) =
            install_prompt_watcher(initial.as_ref(), editor_id, window, cx);

        let mut content = TaskEditContent {
            task_id,
            title_input,
            branch_input,
            branch_validation,
            draft_subtasks: Vec::new(),
            preview_prompt: false,
            settings_open: false,
            notes_open: !notes.is_empty(),
            prompt_state,
            notes_state,
            auto_execute,
            agent_surface,
            focus_handle,
            cached_title,
            // Replaced below by what the built form actually shows.
            saved_snapshot: TaskEditValues::default(),
            project_select,
            base_select,
            run_in,
            lane_select,
            _subscriptions: vec![
                project_sub,
                title_sub,
                branch_sub,
                base_sub,
                lane_sub,
                new_subtask_sub,
                rename_subtask_sub,
                prompt_sub,
                notes_sub,
            ],
            _prompt_watcher,
            _prompt_pump,
            new_subtask_input,
            editing_subtask: None,
            editing_subtask_input,
            disk_copy: None,
            body_scroll_handle: gpui::ScrollHandle::new(),
        };
        // The baseline is what the form shows: a base or lane no longer
        // registered is not selected, and must not read as an edit.
        content.saved_snapshot = content.current_snapshot(cx);
        content
    }

    /// Refresh the tab title from the title input.
    pub(super) fn refresh_task_edit_title(
        &mut self,
        editor_id: TaskEditorId,
        cx: &mut Context<Self>,
    ) {
        let Some(te) = self.task_editor(editor_id) else {
            return;
        };
        let title = te.title_input.read(cx).value().to_string();
        if let Some(te) = self.task_editor_mut(editor_id) {
            te.cached_title = if title.is_empty() {
                crate::surface::strings::command::new_task().into()
            } else {
                SharedString::from(title)
            };
        }
        cx.notify();
    }

    /// User typed into the branch input directly — re-validate.
    pub(super) fn on_task_edit_branch_typed(
        &mut self,
        editor_id: TaskEditorId,
        cx: &mut Context<Self>,
    ) {
        let Some(te) = self.task_editor(editor_id) else {
            return;
        };
        let (branch_text, editable, project) = (
            te.branch_input.read(cx).text().to_string(),
            super::run_in_ops::location_editable(te, cx.global()),
            te.project(cx),
        );
        let validation = self.branch_validation_for(&branch_text, editable, project);
        if let Some(te) = self.task_editor_mut(editor_id) {
            te.branch_validation = validation;
        }
        cx.notify();
    }

    /// The base and run-in pickers name the project's lanes, so another
    /// project leaves both pointing at lanes it does not have: refill them
    /// and drop the picks.
    pub(super) fn on_task_edit_project_changed(
        &mut self,
        editor_id: TaskEditorId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(te) = self.task_editor(editor_id) else {
            return;
        };
        let project = te.project(cx);
        let branch = te.branch_input.read(cx).text().to_string();
        let editable = super::run_in_ops::location_editable(te, cx.global());
        let (base_select, lane_select) = (te.base_select.clone(), te.lane_select.clone());
        let base_options = base_lane_options(self, project);
        let lane_options = super::run_in_ops::lane_options(self, project);
        base_select.update(cx, |state, cx| {
            state.set_items(base_options, window, cx);
            state.set_selected_value(&SharedString::default(), window, cx);
        });
        lane_select.update(cx, |state, cx| {
            state.set_items(lane_options, window, cx);
            state.set_selected_index(None, window, cx);
        });
        let validation = self.branch_validation_for(&branch, editable, project);
        if let Some(te) = self.task_editor_mut(editor_id) {
            te.branch_validation = validation;
        }
        cx.notify();
    }

    /// The editor named `id`, wherever it is held. Every editor lookup goes
    /// through this pair, so a window-close "Save all" reaches an editor that
    /// is not on screen, and a late callback for an editor that is gone finds
    /// nothing.
    pub(in crate::workspace) fn task_editor(&self, id: TaskEditorId) -> Option<&TaskEditContent> {
        self.pages
            .tasks
            .detail
            .as_ref()
            .filter(|detail| detail.id == id)
            .map(|detail| &detail.editor)
    }

    pub(in crate::workspace) fn task_editor_mut(
        &mut self,
        id: TaskEditorId,
    ) -> Option<&mut TaskEditContent> {
        self.pages
            .tasks
            .detail
            .as_mut()
            .filter(|detail| detail.id == id)
            .map(|detail| &mut detail.editor)
    }

    pub(super) fn open_editor_task_chat(
        &mut self,
        editor_id: TaskEditorId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self
            .task_editor(editor_id)
            .and_then(|te| te.task_id.clone())
        {
            self.open_task_chat(&id, window, cx);
        }
    }
}

/// Every project open in this window, keyed by `ProjectUuid`.
fn project_options(ws: &Workspace) -> Vec<SelectOption> {
    ws.projects
        .iter()
        .map(|p| SelectOption::new(super::state::project_value(p.uuid), p.name.clone()))
        .collect()
}

/// Build the `base_select` option list from `project`'s lanes. The
/// leading empty-string option is the "no explicit base — the project's
/// base branch at `start_task` time" sentinel; remaining entries are keyed
/// by absolute path so `commit_task_form` can round-trip the user's
/// pick back into `Task::base_worktree_path: Option<PathBuf>`.
fn base_lane_options(ws: &Workspace, project: Option<ProjectUuid>) -> Vec<SelectOption> {
    let lanes = ws.task_project_lanes(project);
    let mut options = Vec::with_capacity(lanes.len() + 1);
    options.push(SelectOption::new(
        "",
        crate::surface::strings::task::edit_base_active_label(),
    ));
    for w in lanes {
        let Some(path_str) = w.path.to_str() else {
            continue;
        };
        // `SharedString::from(&str)` doesn't exist — convert to owned
        // `String` so the resulting option is `'static` and the
        // workspace borrow can end at the end of this function.
        options.push(SelectOption::new(path_str.to_string(), w.display_name()));
    }
    options
}
