//! TaskEdit pane lifecycle — builder, open / find, branch validation,
//! save / discard / start dispatchers.
//!
//! The pane itself is rendered by `render::task_edit_pane`; this
//! module owns the *operations* (open, save, validate, find existing)
//! that the renderer + status_pill + Tasks-tab row click dispatch into.
//!
//! Branch validation runs the shared `daruda_core::git` rule walk and
//! renders the rule it broke, so the form's inline diagnostic and the
//! silent filter behind `sanitize_branch_name` can never disagree on what
//! git accepts.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;
use daruda_store::tasks::{SubTask, Task, TaskAgentSurface, TaskId, TaskRunIn, random_branch_name};
use gpui::{AppContext as _, BorrowAppContext as _, Context, Focusable as _, SharedString, Window};

use crate::ui::select::{SelectOption, state_with_options};
use crate::ui::{InputEvent, InputState, make_markdown_prose_state};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::{
    BranchValidation, Pane, PaneContent, RunInChoice, TaskEditContent, TaskEditSnapshot,
};
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

/// Validate a branch-input string, reporting which rule it broke so the
/// form can show a precise red label. An empty field is not an error — a
/// draft then gets its default `task-<id>` branch, a saved task keeps its own.
pub(super) fn validate_branch(text: &str) -> BranchValidation {
    match daruda_core::git::validate_branch_name(text) {
        Ok(_) => BranchValidation::Valid,
        Err(daruda_core::git::BranchNameRule::Empty) => BranchValidation::Empty,
        Err(rule) => BranchValidation::Invalid {
            reason: SharedString::from(crate::surface::strings::branch_rule_reason(rule)),
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
                .placeholder(crate::surface::strings::task_edit_title_placeholder());
            if !title_for_default.is_empty() {
                s = s.default_value(title_for_default);
            }
            s
        });

        let branch_for_default = branch_name.clone();
        let branch_input = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state)
                .placeholder(crate::surface::strings::task_edit_branch_placeholder());
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
            crate::surface::strings::task_edit_prompt_placeholder(),
            crate::ui::theme::TASK_EDIT_PROMPT_ROWS,
            window,
            cx,
        );
        let notes_state = make_markdown_prose_state(
            &notes,
            crate::surface::strings::task_edit_notes_placeholder(),
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
                .placeholder(crate::surface::strings::task_subtask_add_placeholder())
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
            crate::surface::strings::command_new_task().into()
        } else {
            SharedString::from(title.clone())
        };

        let editable = initial
            .as_ref()
            .is_none_or(|t| matches!(t.state, daruda_store::tasks::TaskState::Backlog));
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

        let saved_snapshot = TaskEditSnapshot {
            draft_subtasks: Vec::new(),
            title: title.clone(),
            branch: branch_name.clone(),
            prompt: normalize_newlines(&prompt),
            notes: normalize_newlines(&notes),
            auto_execute,
            agent_surface,
            base_value: base_initial
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            run_in,
            // Read back, not `lane_initial`: a lane no longer registered is
            // not selected, and the baseline must match what the form shows.
            lane_value: lane_select
                .read(cx)
                .selected_value()
                .map(|v| v.to_string())
                .unwrap_or_default(),
        };

        // Running tasks watch immediately; Start attaches the watcher once
        // it has materialized the task's prompt file.
        let (_prompt_watcher, _prompt_pump) =
            install_prompt_watcher(initial.as_ref(), pane_id, window, cx);

        Pane {
            id: pane_id,
            content: PaneContent::TaskEditPane(TaskEditContent {
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
                saved_snapshot,
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
            }),
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
                crate::surface::strings::command_new_task().into()
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
    fn task_edit_content_mut_for(&mut self, pane_id: PaneId) -> Option<&mut TaskEditContent> {
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

    /// Dynamically install the prompt-file FS watcher on a TaskEdit
    /// pane that's still open when its task transitions Backlog →
    /// Running. At pane-open time the lane didn't exist
    /// yet so `install_prompt_watcher` returned `None`; `start_task`
    /// just wrote the file, so the watcher can finally subscribe.
    /// No-op when the pane is closed, when there's already a watcher
    /// attached, or when the task still has no lane.
    pub(in crate::workspace) fn attach_prompt_watcher_if_pane_open(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Starting a task activates its new lane; the authoring tab stays
        // in the source lane, and may be open in more than one lane.
        let panes: Vec<_> = self
            .main_area
            .runtimes
            .values()
            .flat_map(|runtime| runtime.panes.iter())
            .filter_map(|pane| {
                let te = pane.task_edit_content()?;
                (te.task_id.as_deref() == Some(task_id) && te._prompt_watcher.is_none())
                    .then_some(pane.id)
            })
            .collect();
        let task = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .get(task_id)
            .cloned();
        for pane_id in panes {
            let (handle, pump) = install_prompt_watcher(task.as_ref(), pane_id, window, cx);
            if let Some(te) = self.task_edit_content_mut_for(pane_id) {
                te._prompt_watcher = handle;
                te._prompt_pump = pump;
            }
        }
    }

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
        if form.title.trim().is_empty() {
            return None;
        }
        // `None` once started: where the task runs is then fixed.
        let run_in = if form.editable {
            Some(self.commit_task_run_in(pane_id, &form)?)
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
        let base_path: Option<std::path::PathBuf> = if form.base_value.is_empty()
            || matches!(run_in, Some(TaskRunIn::ExistingLane { .. }))
        {
            None
        } else {
            Some(std::path::PathBuf::from(&form.base_value))
        };

        // The rule walk trims, so the value it accepted is the one stored.
        let branch = daruda_core::git::validate_branch_name(&form.branch)
            .ok()
            .map(str::to_owned);

        let task_id = match &form.task_id {
            Some(id) => {
                cx.global::<crate::agent::tasks_global::GlobalTasks>()
                    .get(id)?;
                self.update_task(
                    id,
                    crate::workspace::right_dock::task_ops::TaskEdits {
                        title: form.title.clone(),
                        prompt: form.prompt.clone(),
                        notes: form.notes.clone(),
                        auto_execute: form.auto_execute,
                        agent_surface: form.agent_surface,
                        base_worktree_path: base_path.clone(),
                        branch,
                        run_in,
                    },
                    cx,
                );
                id.clone()
            }
            None => {
                let mut task = daruda_store::tasks::Task::new(
                    form.title.clone(),
                    form.prompt.clone(),
                    base_path.clone(),
                );
                if let Some(branch) = branch {
                    task.branch_name = branch;
                }
                task.run_in = run_in.unwrap_or_default();
                task.subtasks = form.draft_subtasks.clone();
                task.notes = form.notes.clone();
                task.auto_execute = form.auto_execute;
                task.agent_surface = form.agent_surface;
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
            te.saved_snapshot = crate::workspace::main_area::pane::TaskEditSnapshot {
                draft_subtasks: Vec::new(),
                title: form.title.clone(),
                branch: form.branch.clone(),
                prompt: normalize_newlines(&form.prompt),
                notes: normalize_newlines(&form.notes),
                auto_execute: form.auto_execute,
                agent_surface: form.agent_surface,
                base_value: form.base_value.clone(),
                run_in: form.run_in,
                lane_value: form.lane_value.clone(),
            };
        }

        Some(task_id)
    }

    /// The location a still-editable form commits to, or `None` when it is
    /// not savable. The branch is checked again here because a lane created
    /// after the form opened can have taken it since the last keystroke.
    fn commit_task_run_in(
        &mut self,
        pane_id: PaneId,
        form: &TaskEditFormSnapshot,
    ) -> Option<TaskRunIn> {
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
    fn read_task_edit_form(
        &self,
        pane_id: PaneId,
        cx: &Context<Self>,
    ) -> Option<TaskEditFormSnapshot> {
        let te = self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .find(|p| p.id == pane_id)?
            .task_edit_content()?;
        Some(TaskEditFormSnapshot {
            draft_subtasks: te.draft_subtasks.clone(),
            task_id: te.task_id.clone(),
            title: te.title_input.read(cx).text().to_string(),
            branch: te.branch_input.read(cx).text().to_string(),
            prompt: te.prompt_state.read(cx).text().to_string(),
            notes: te.notes_state.read(cx).text().to_string(),
            auto_execute: te.auto_execute,
            agent_surface: te.agent_surface,
            editable: super::run_in_ops::location_editable(te, cx.global()),
            run_in: te.run_in,
            lane_value: te.lane_value(cx),
            base_value: te
                .base_select
                .read(cx)
                .selected_value()
                .map(|v| v.to_string())
                .unwrap_or_default(),
        })
    }
}

/// Plain-data form snapshot used by `save_task_edit_pane` so the save
/// path doesn't keep a borrow on `self.active_runtime().panes` past the read step.
struct TaskEditFormSnapshot {
    draft_subtasks: Vec<SubTask>,
    task_id: Option<TaskId>,
    title: String,
    branch: String,
    prompt: String,
    notes: String,
    auto_execute: bool,
    agent_surface: TaskAgentSurface,
    /// Whether the task has yet to start, so its location may still change.
    editable: bool,
    run_in: RunInChoice,
    lane_value: String,
    /// Selected `base_select` value — empty string sentinel for "use
    /// active lane", otherwise an absolute path string.
    base_value: String,
}

/// CRLF → LF for dirty-comparison snapshots. External editors (vim,
/// VS Code on Windows) may rewrite the prompt file with CRLF; we
/// don't want that to register as a user edit.
pub(super) fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// The on-disk prompt file for `task` — only meaningful once the task has
/// been started (i.e. has a lane). Returns `None` for Backlog / drafts.
fn prompt_file_path_for(task: &Task) -> Option<std::path::PathBuf> {
    let wt = task.state.worktree_path()?;
    Some(daruda_store::tasks::existing_prompt_file_path(task, wt))
}

/// Install the watcher and pump when the task has a prompt file on disk.
fn install_prompt_watcher(
    initial: Option<&Task>,
    pane_id: PaneId,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> (
    Option<crate::workspace::main_area::prompt_watcher::PromptFileWatcherHandle>,
    Option<gpui::Task<()>>,
) {
    let Some(task) = initial else {
        return (None, None);
    };
    let Some(path) = prompt_file_path_for(task) else {
        return (None, None);
    };
    if !path.exists() {
        return (None, None);
    }

    let (events_rx, handle) = crate::workspace::main_area::prompt_watcher::spawn(path.clone());
    let path_for_pump = path.clone();
    let pump = cx.spawn_in(window, async move |this, cx| {
        const POLL: std::time::Duration = std::time::Duration::from_millis(100);
        'outer: loop {
            cx.background_executor().timer(POLL).await;
            loop {
                match events_rx.try_recv() {
                    Ok(()) => {
                        // Coalesce multiple debounce-window signals so a
                        // burst still results in a single dispatch.
                        while events_rx.try_recv().is_ok() {}
                        let path_for_dispatch = path_for_pump.clone();
                        if this
                            .update_in(cx, |ws, window, cx| {
                                ws.handle_prompt_file_changed(
                                    pane_id,
                                    path_for_dispatch,
                                    window,
                                    cx,
                                );
                            })
                            .is_err()
                        {
                            break 'outer;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break 'outer,
                }
            }
        }
    });

    (Some(handle), Some(pump))
}

impl Workspace {
    /// Dispatched by the prompt-file watcher when an external editor
    /// rewrites `<wt>/.daruda/task-<id>.md`. Reloads the editor
    /// silently when the pane is clean; surfaces a conflict prompt
    /// (Use disk version / Keep my version / Diff) when the pane is
    /// dirty.
    pub(super) fn handle_prompt_file_changed(
        &mut self,
        pane_id: PaneId,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A transient atomic-rename mid-flight is normal here, but a
        // persistent failure (permission flip, unmount) silently
        // wedges the watcher — leave a single Info breadcrumb so the
        // condition is visible in the NDJSON log without yelling at
        // the user via toast.
        let disk_content = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                LogWriter::log(
                    ErrorReport::new(crate::surface::strings::error_prompt_watcher_read_failed())
                        .severity(ErrorSeverity::Info)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("path", redact_home(&path))
                        .dedup("tasks.prompt_watcher.read")
                        .build(),
                );
                return;
            }
        };

        let Some(pane) = self
            .main_area
            .runtimes
            .values()
            .flat_map(|runtime| runtime.panes.iter())
            .find(|p| p.id == pane_id)
        else {
            return;
        };
        let Some(te) = pane.task_edit_content() else {
            return;
        };
        let prompt_entity = te.prompt_state.clone();
        let title = pane.title(cx);
        let is_dirty = te.is_dirty(cx);

        // If the disk content already matches what's in the editor
        // (modulo CRLF), this is almost certainly a save-side echo
        // from our own `write_prompt_file`. Don't bother the user —
        // just re-baseline so the pane stays clean.
        let editor_normalized =
            normalize_newlines(prompt_entity.read(cx).text().to_string().as_str());
        let disk_normalized = normalize_newlines(&disk_content);
        if editor_normalized == disk_normalized {
            if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
                te.saved_snapshot.prompt = disk_normalized;
            }
            return;
        }

        if !is_dirty {
            self.reload_prompt_from_disk(pane_id, prompt_entity, disk_content, window, cx);
            return;
        }

        // Dirty — surface a 3-button platform prompt and route the
        // answer back into reload / no-op / diff.
        let heading = format!(
            "{}{}{}",
            crate::surface::strings::PROMPT_WATCHER_HEADING_PREFIX,
            title,
            crate::surface::strings::prompt_watcher_heading_suffix(),
        );
        let prompt_detail = crate::surface::strings::prompt_watcher_detail();
        let prompt_use_disk = crate::surface::strings::prompt_watcher_use_disk();
        let prompt_keep_mine = crate::surface::strings::prompt_watcher_keep_mine();
        let prompt_diff = crate::surface::strings::prompt_watcher_diff();
        let receiver = window.prompt(
            gpui::PromptLevel::Warning,
            &heading,
            Some(prompt_detail.as_str()),
            &[
                prompt_use_disk.as_str(),
                prompt_keep_mine.as_str(),
                prompt_diff.as_str(),
            ],
            cx,
        );

        let path_for_diff = path.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
            // SILENT-OK: user may close window before save-dialog answer arrives
            let _ = this.update_in(cx, |this, window, cx| match answer {
                0 => {
                    let Some(pane) = this.active_runtime().panes.iter().find(|p| p.id == pane_id)
                    else {
                        return;
                    };
                    let Some(te) = pane.task_edit_content() else {
                        return;
                    };
                    let prompt_entity = te.prompt_state.clone();
                    this.reload_prompt_from_disk(
                        pane_id,
                        prompt_entity,
                        disk_content.clone(),
                        window,
                        cx,
                    );
                }
                1 => {} // Keep my version — leave editor untouched
                2 => {
                    // Split the TaskEdit pane's tab to the right with
                    // the disk version so the user sees both at once.
                    this.open_disk_file_for_diff(pane_id, path_for_diff.clone(), window, cx);
                }
                _ => {}
            });
        })
        .detach();
    }

    /// Overwrite the pane's prompt editor with `content` and rebaseline
    /// the dirty snapshot so the pane no longer reads as dirty.
    fn reload_prompt_from_disk(
        &mut self,
        pane_id: PaneId,
        prompt_entity: gpui::Entity<gpui_component::input::InputState>,
        content: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        prompt_entity.update(cx, |state, cx| state.set_value(content.clone(), window, cx));
        if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.saved_snapshot.prompt = normalize_newlines(&content);
        }
        cx.notify();
    }

    /// Open `<wt>/.daruda/task-<id>.md` in a fresh file viewer
    /// tab (`[📄 Open file]` button). No-op for tasks that
    /// haven't been started yet — Backlog tasks have no lane
    /// path, and Started tasks whose prompt file disappeared (e.g.
    /// manual delete) silently bail rather than open a viewer onto a
    /// non-existent file. The button itself is disabled in those
    /// states so this is defensive only.
    pub(super) fn open_task_prompt_file(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .get(task_id)
            .cloned()
        else {
            return;
        };
        let Some(path) = prompt_file_path_for(&task) else {
            return;
        };
        if !path.exists() {
            let report = ErrorReport::new(crate::surface::strings::error_prompt_file_not_found())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("path", redact_home(&path))
                .dedup("tasks.open_prompt_file.missing")
                .build();
            self.report_error(report, cx);
            return;
        }
        let Some(wt_id) = self.lane_containing(&path) else {
            let report =
                ErrorReport::new(crate::surface::strings::error_prompt_file_outside_lanes())
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("path", redact_home(&path))
                    .dedup("tasks.open_prompt_file.no_lane")
                    .build();
            self.report_error(report, cx);
            return;
        };
        let wt_ref = daruda_store::project::LaneRef {
            project: self.active.project,
            lane: wt_id,
        };
        self.open_files_entry(
            wt_ref,
            path,
            crate::workspace::main_area::tab_ops::OpenIntent::Enter,
            window,
            cx,
        );
    }

    /// Helper used by the conflict prompt's `[Diff]` branch.
    /// Opens `path` in a file viewer pane *split to the right of* the
    /// owning TaskEdit pane so the user sees the in-pane editor on
    /// the left and the disk version on the right simultaneously
    /// The two-pane layout lets the user compare in-pane edits against
    /// the on-disk version side-by-side. Falls back silently when the
    /// path isn't inside any known lane.
    fn open_disk_file_for_diff(
        &mut self,
        pane_id: PaneId,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(wt) = self.lane_containing(&path) else {
            return;
        };
        self.open_file_split_right(wt, path, pane_id, window, cx);
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
        crate::surface::strings::task_edit_base_active_label(),
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

impl Workspace {
    /// The active lane `path` lies in — the deepest, so a worktree nested
    /// inside another lane's checkout wins over its parent. Spellings are
    /// compared as one place, since a prompt path may come through a symlink.
    fn lane_containing(&self, path: &std::path::Path) -> Option<daruda_store::project::LaneId> {
        self.active_lanes()
            .iter()
            .filter(|w| daruda_core::path::is_within(path, &w.path))
            .max_by_key(|w| w.path.components().count())
            .map(|w| w.id)
    }
}
