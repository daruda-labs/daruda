//! TaskEdit pane lifecycle — builder, open / find, branch validation, and
//! the accessors the sibling `save_ops` / `prompt_file_ops` reach panes by.
//!
//! The pane itself is rendered by `render::task_edit_pane`; this
//! module owns the *operations* (open, validate, find existing)
//! that the renderer + status_pill + Tasks-tab row click dispatch into.
//!
//! Branch validation runs the shared `daruda_core::git` rule walk and
//! renders the rule it broke, so the form's inline diagnostic and the
//! silent filter behind `sanitize_branch_name` can never disagree on what
//! git accepts.

use daruda_store::tasks::{Task, TaskAgentSurface, TaskId, random_branch_name};
use gpui::{AppContext as _, Context, Focusable as _, SharedString, Window};

use super::prompt_file_ops::install_prompt_watcher;
use super::state::{BranchValidation, TaskEditContent, TaskEditValues};
use crate::ui::select::{SelectOption, state_with_options};
use crate::ui::{InputEvent, InputState, make_markdown_prose_state};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::{Pane, PaneContent};
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

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
    /// Open (or focus) the TaskEdit pane for `task_id`. `None` opens a
    /// fresh draft pane. Same task → second open re-focuses the
    /// existing pane instead of creating a duplicate.
    pub(in crate::workspace) fn open_task_edit_pane(
        &mut self,
        task_id: Option<TaskId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = task_id.as_deref()
            && let Some(existing) = self.find_task_edit_pane(id)
        {
            self.focus_pane(existing, window, cx);
            return;
        }

        let initial = task_id.as_deref().and_then(|id| {
            cx.global::<crate::agent::tasks_global::GlobalTasks>()
                .get(id)
                .cloned()
        });

        let pane = self.create_task_edit_pane(task_id, initial, window, cx);
        let pane_id = pane.id;
        let tab_id = self.alloc_id();
        self.active_runtime_mut().panes.push(pane);
        self.active_runtime_mut()
            .tabs
            .push(crate::workspace::main_area::pane::TabEntry {
                id: tab_id,
                layout: PaneLayout::Pane(pane_id),
                last_focused_pane: pane_id,
                user_label: None,
            });
        let cur_tab = self.active_runtime().active_tab_index;
        self.active_runtime_mut().tab_history.push(cur_tab);
        let last_tab = self.active_runtime().tabs.len() - 1;
        self.active_runtime_mut().active_tab_index = last_tab;
        self.set_focused_pane(pane_id, window, cx);
        self.bump_activity(pane_id);
        self.focus_pane(pane_id, window, cx);
        if let Some(te) = self.task_edit_content_for_pane(pane_id) {
            te.title_input.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// Return the `PaneId` of the existing TaskEdit pane tied to
    /// `task_id`, if any. Drafts (`task_id = None`) are never
    /// deduplicated — each `[+ New]` click is a fresh draft.
    pub(super) fn find_task_edit_pane(&self, task_id: &str) -> Option<PaneId> {
        self.active_runtime()
            .panes
            .iter()
            .find_map(|p| match &p.content {
                PaneContent::TaskEditPane(te) if te.task_id.as_deref() == Some(task_id) => {
                    Some(p.id)
                }
                _ => None,
            })
    }

    fn create_task_edit_pane(
        &mut self,
        task_id: Option<TaskId>,
        initial: Option<Task>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Pane {
        let pane_id = self.alloc_id();

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
                    this.refresh_task_edit_title(pane_id, cx);
                }
            },
        );
        let branch_sub = cx.subscribe_in(
            &branch_input,
            window,
            move |this, _inp, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.on_task_edit_branch_typed(pane_id, cx);
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
                    this.submit_new_subtask(pane_id, window, cx);
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
                    this.commit_rename_subtask(pane_id, cx);
                }
            },
        );

        // Base-lane selector. The leading
        // empty-string sentinel maps to `Task::base_worktree_path ==
        // None`; the remaining options are registered lanes keyed
        // by absolute path. Building the option list once here keeps
        // the dropdown stable across rerenders — re-deriving on every
        // frame would burn allocations and reset list-search state.
        let base_options = base_lane_options(self);
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
        let branch_validation = self.branch_validation_for(&branch_name, editable);
        let run_in = super::run_in_ops::run_in_choice(initial.as_ref());
        let lane_initial = super::run_in_ops::initial_lane(initial.as_ref(), self);
        let lane_select = cx.new(|cx| {
            state_with_options(
                super::run_in_ops::lane_options(self),
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
            install_prompt_watcher(initial.as_ref(), pane_id, window, cx);

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
            base_select,
            run_in,
            lane_select,
            _subscriptions: vec![
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
            body_scroll_handle: gpui::ScrollHandle::new(),
        };
        // The baseline is what the form shows: a base or lane no longer
        // registered is not selected, and must not read as an edit.
        content.saved_snapshot = content.current_snapshot(cx);
        Pane {
            id: pane_id,
            content: PaneContent::TaskEditPane(content),
        }
    }

    /// Refresh the tab title from the title input.
    pub(super) fn refresh_task_edit_title(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let Some(te) = self.task_edit_content_for_pane(pane_id) else {
            return;
        };
        let title = te.title_input.read(cx).value().to_string();
        if let Some(te) = self.task_edit_content_mut_for(pane_id) {
            te.cached_title = if title.is_empty() {
                crate::surface::strings::command::new_task().into()
            } else {
                SharedString::from(title)
            };
        }
        cx.notify();
    }

    /// User typed into the branch input directly — re-validate.
    pub(super) fn on_task_edit_branch_typed(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let (branch_text, editable) =
            match self.active_runtime().panes.iter().find(|p| p.id == pane_id) {
                Some(p) => match p.task_edit_content() {
                    Some(te) => (
                        te.branch_input.read(cx).text().to_string(),
                        super::run_in_ops::location_editable(te, cx.global()),
                    ),
                    None => return,
                },
                None => return,
            };
        let validation = self.branch_validation_for(&branch_text, editable);
        if let Some(te) = self.task_edit_content_mut_for(pane_id) {
            te.branch_validation = validation;
        }
        cx.notify();
    }

    /// Searches every lane: a window-close "Save all" commits a pane parked
    /// in a lane that is not on screen.
    pub(super) fn task_edit_content_mut_for(
        &mut self,
        pane_id: PaneId,
    ) -> Option<&mut TaskEditContent> {
        self.main_area
            .runtimes
            .values_mut()
            .flat_map(|rt| rt.panes.iter_mut())
            .find(|p| p.id == pane_id)?
            .task_edit_content_mut()
    }

    /// Public counterpart used by the renderer's click handlers (e.g.
    /// the auto-execute checkbox) to flip a field on the focused pane
    /// without going through a private helper.
    pub(super) fn task_edit_content_mut_for_pane(
        &mut self,
        pane_id: PaneId,
    ) -> Option<&mut TaskEditContent> {
        self.task_edit_content_mut_for(pane_id)
    }

    /// Immutable lookup — returns the TaskEdit content tied to
    /// `pane_id`, if any. Used by listeners that only need to read a
    /// field (e.g. the prompt-header "Open file" button reading
    /// `task_id` to dispatch `open_task_prompt_file`).
    pub(super) fn task_edit_content_for_pane(&self, pane_id: PaneId) -> Option<&TaskEditContent> {
        let pane = self
            .main_area
            .runtimes
            .values()
            .flat_map(|runtime| runtime.panes.iter())
            .find(|p| p.id == pane_id)?;
        match &pane.content {
            PaneContent::TaskEditPane(te) => Some(te),
            _ => None,
        }
    }

    pub(super) fn open_editor_task_chat(
        &mut self,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self
            .task_edit_content_for_pane(pane)
            .and_then(|te| te.task_id.clone())
        {
            self.open_task_chat(&id, window, cx);
        }
    }
}

/// Build the `base_select` option list from the workspace's current
/// lanes. The leading empty-string option is the "no explicit
/// base — defer to the active lane at `start_task` time"
/// sentinel; remaining entries are keyed by absolute path so
/// `commit_task_edit_pane` can round-trip the user's pick back into
/// `Task::base_worktree_path: Option<PathBuf>`.
fn base_lane_options(ws: &Workspace) -> Vec<SelectOption> {
    let lanes = ws.active_lanes();
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
