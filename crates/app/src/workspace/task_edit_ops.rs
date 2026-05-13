//! TaskEdit pane lifecycle — builder, open / find, branch validation,
//! save / discard / start dispatchers (R-19 + R-25 + I-12).
//!
//! The pane itself is rendered by `right_panel::task_edit_pane`; this
//! module owns the *operations* (open, save, validate, find existing)
//! that the renderer + status_pill + Tasks-tab row click dispatch into.
//!
//! Branch validation mirrors `daruda_store::tasks::sanitize_branch_name`
//! but returns a `BranchValidation` enum with the specific failure
//! reason so the form can display "cannot contain space" etc. inline.
//! The two implementations must stay in sync — `sanitize_branch_name`
//! is the source of truth for "what git accepts" and `validate_branch`
//! is the user-facing diagnostic surface.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;
use daruda_store::tasks::{Task, TaskId, branch::derive_branch_name};
use gpui::{AppContext as _, BorrowAppContext as _, Context, Focusable as _, SharedString, Window};

use super::Workspace;
use super::layout::{PaneId, PaneLayout};
use super::pane::{BranchValidation, Pane, PaneContent, TaskEditContent, TaskEditSnapshot};
use crate::ui::select::{SelectOption, state_with_options};
use crate::ui::{InputEvent, InputState, make_markdown_prose_state};

/// Validate a branch-input string against the same rules as
/// `daruda_store::tasks::sanitize_branch_name`, but report which
/// rule was violated so the form can show a precise red label.
pub(in crate::workspace) fn validate_branch(text: &str) -> BranchValidation {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return BranchValidation::Empty;
    }
    if trimmed.contains("..") {
        return BranchValidation::Invalid {
            reason: "cannot contain '..'".into(),
        };
    }
    if trimmed.starts_with('/') || trimmed.ends_with('/') {
        return BranchValidation::Invalid {
            reason: "cannot start or end with '/'".into(),
        };
    }
    if trimmed.starts_with('.') || trimmed.ends_with('.') {
        return BranchValidation::Invalid {
            reason: "cannot start or end with '.'".into(),
        };
    }
    for c in trimmed.chars() {
        if c.is_control() {
            return BranchValidation::Invalid {
                reason: "contains a control character".into(),
            };
        }
        match c {
            ' ' => {
                return BranchValidation::Invalid {
                    reason: "cannot contain spaces".into(),
                };
            }
            ':' | '~' | '^' | '?' | '*' | '[' | '\\' => {
                return BranchValidation::Invalid {
                    reason: SharedString::from(format!("cannot contain '{c}'")),
                };
            }
            _ => {}
        }
    }
    BranchValidation::Valid
}

impl Workspace {
    /// Open (or focus) the TaskEdit pane for `task_id`. `None` opens a
    /// fresh draft pane. Same task → second open re-focuses the
    /// existing pane (I-6) instead of creating a duplicate.
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
        self.panes.push(pane);
        self.tabs.push(super::pane::Tab {
            id: tab_id,
            layout: PaneLayout::Leaf(pane_id),
            last_focused_pane: pane_id,
            user_label: None,
        });
        self.tab_history.push(self.active_tab_index);
        self.active_tab_index = self.tabs.len() - 1;
        self.focused_pane_id = pane_id;
        self.bump_activity(pane_id);
        self.focus_pane(pane_id, window, cx);
        cx.notify();
    }

    /// Return the `PaneId` of the existing TaskEdit pane tied to
    /// `task_id`, if any. Drafts (`task_id = None`) are never
    /// deduplicated — each `[+ New]` click is a fresh draft.
    pub(in crate::workspace) fn find_task_edit_pane(&self, task_id: &str) -> Option<PaneId> {
        self.panes.iter().find_map(|p| match &p.content {
            PaneContent::TaskEdit(te) if te.task_id.as_deref() == Some(task_id) => Some(p.id),
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

        let (title, prompt, notes, branch_name, auto_execute) = match &initial {
            Some(t) => (
                t.title.clone(),
                t.prompt.clone(),
                t.notes.clone(),
                t.branch_name.clone(),
                t.auto_execute,
            ),
            None => (
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                true,
            ),
        };

        let title_for_default = title.clone();
        let title_input = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state).placeholder("Task title");
            if !title_for_default.is_empty() {
                s = s.default_value(title_for_default);
            }
            s
        });

        let branch_for_default = branch_name.clone();
        let branch_input = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state).placeholder("branch-name");
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
        let prompt_state = make_markdown_prose_state(&prompt, "Describe the task…", 20, window, cx);
        let notes_state = make_markdown_prose_state(&notes, "Notes (markdown)", 4, window, cx);

        let title_sub = cx.subscribe_in(
            &title_input,
            window,
            move |this, _inp, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.refresh_task_edit_branch(pane_id, window, cx);
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

        // R-21 subtask inputs — one for the trailing `[+ Add subtask…]`
        // row, one shared across all inline-rename rows. Both go
        // through `gpui_component::Input` (IME-verified equivalent of
        // the old daruda TextInput per ADR S1).
        let new_subtask_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(crate::surface::strings::TASK_SUBTASK_ADD_PLACEHOLDER)
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

        // Base-worktree selector (R-19 / C-1). The leading
        // empty-string sentinel maps to `Task::base_worktree_path ==
        // None`; the remaining options are registered worktrees keyed
        // by absolute path. Building the option list once here keeps
        // the dropdown stable across rerenders — re-deriving on every
        // frame would burn allocations and reset list-search state.
        let base_options = base_worktree_options(self);
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
            "New task".into()
        } else {
            SharedString::from(title.clone())
        };

        let branch_validation = validate_branch(&branch_name);

        let saved_snapshot = TaskEditSnapshot {
            title: title.clone(),
            branch: branch_name.clone(),
            prompt: normalize_newlines(&prompt),
            notes: normalize_newlines(&notes),
            auto_execute,
            base_value: base_initial
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or_default(),
        };

        // R-20: install the FS watcher when the task has a worktree
        // and the prompt file is already on disk. Backlog tasks have
        // no worktree yet; drafts have neither. Subsequent
        // `start_task` runs that materialise the worktree are out of
        // scope for this PR — reopening the pane after Start picks
        // up the watcher.
        let (_prompt_watcher, _prompt_pump) =
            install_prompt_watcher(initial.as_ref(), pane_id, window, cx);

        Pane {
            id: pane_id,
            content: PaneContent::TaskEdit(TaskEditContent {
                task_id,
                title_input,
                branch_input,
                branch_override: !branch_name.is_empty()
                    && initial
                        .as_ref()
                        .map(|t| derive_branch_name(&t.title, &t.id) != branch_name)
                        .unwrap_or(false),
                branch_validation,
                prompt_state,
                notes_state,
                auto_execute,
                focus_handle,
                cached_title,
                saved_snapshot,
                base_select,
                _subscriptions: vec![
                    title_sub,
                    branch_sub,
                    base_sub,
                    new_subtask_sub,
                    rename_subtask_sub,
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

    /// Auto-derive branch from title — fires from the title input's
    /// `Changed` event. No-op once the user has manually edited the
    /// branch (I-12 / `branch_override = true`).
    pub(in crate::workspace) fn refresh_task_edit_branch(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(te) = pane.task_edit_content() else {
            return;
        };
        if te.branch_override {
            return;
        }
        let title = te.title_input.read(cx).value().to_string();
        // ULID-shaped id for draft panes: real tasks reuse their own
        // ULID, drafts use the task_id or a temporary "draft" stamp
        // so the suffix is stable across keystrokes within a session.
        let ulid_stamp = te
            .task_id
            .clone()
            .unwrap_or_else(|| "draftdraftdraftdraft".to_string());
        let derived = derive_branch_name(&title, &ulid_stamp);
        let branch_entity = te.branch_input.clone();
        branch_entity.update(cx, |inp, cx_state| {
            inp.set_value(derived.clone(), window, cx_state)
        });
        if let Some(te) = self.task_edit_content_mut_for(pane_id) {
            te.branch_validation = validate_branch(&derived);
            te.cached_title = if title.is_empty() {
                "New task".into()
            } else {
                SharedString::from(title)
            };
        }
        cx.notify();
    }

    /// User typed into the branch input directly — flip
    /// `branch_override` so subsequent title edits stop overwriting
    /// the user's value, and re-validate.
    pub(in crate::workspace) fn on_task_edit_branch_typed(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let branch_text = match self.panes.iter().find(|p| p.id == pane_id) {
            Some(p) => match p.task_edit_content() {
                Some(te) => te.branch_input.read(cx).text().to_string(),
                None => return,
            },
            None => return,
        };
        if let Some(te) = self.task_edit_content_mut_for(pane_id) {
            te.branch_override = true;
            te.branch_validation = validate_branch(&branch_text);
        }
        cx.notify();
    }

    fn task_edit_content_mut_for(&mut self, pane_id: PaneId) -> Option<&mut TaskEditContent> {
        self.panes
            .iter_mut()
            .find(|p| p.id == pane_id)?
            .task_edit_content_mut()
    }

    /// Public counterpart used by the renderer's click handlers (e.g.
    /// the auto-execute checkbox) to flip a field on the focused pane
    /// without going through a private helper.
    pub(in crate::workspace) fn task_edit_content_mut_for_pane(
        &mut self,
        pane_id: PaneId,
    ) -> Option<&mut TaskEditContent> {
        self.task_edit_content_mut_for(pane_id)
    }

    /// Immutable lookup — returns the TaskEdit content tied to
    /// `pane_id`, if any. Used by listeners that only need to read a
    /// field (e.g. the prompt-header "Open file" button reading
    /// `task_id` to dispatch `open_task_prompt_file`).
    pub(in crate::workspace) fn task_edit_content_for_pane(
        &self,
        pane_id: PaneId,
    ) -> Option<&TaskEditContent> {
        let pane = self.panes.iter().find(|p| p.id == pane_id)?;
        match &pane.content {
            PaneContent::TaskEdit(te) => Some(te),
            _ => None,
        }
    }

    /// Dynamically install the prompt-file FS watcher on a TaskEdit
    /// pane that's still open when its task transitions Backlog →
    /// Running (R-20). At pane-open time the worktree didn't exist
    /// yet so `install_prompt_watcher` returned `None`; `start_task`
    /// just wrote the file, so the watcher can finally subscribe.
    /// No-op when the pane is closed, when there's already a watcher
    /// attached, or when the task still has no worktree.
    pub(in crate::workspace) fn attach_prompt_watcher_if_pane_open(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane_id) = self.find_task_edit_pane(task_id) else {
            return;
        };
        let already_attached = self
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.task_edit_content())
            .map(|te| te._prompt_watcher.is_some())
            .unwrap_or(true);
        if already_attached {
            return;
        }
        let task = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .get(task_id)
            .cloned();
        let (handle, pump) = install_prompt_watcher(task.as_ref(), pane_id, window, cx);
        if let Some(te) = self.task_edit_content_mut_for(pane_id) {
            te._prompt_watcher = handle;
            te._prompt_pump = pump;
        }
    }

    /// `[+ Add subtask…]` row submitted (Enter). Routes the text into
    /// `add_subtask`, then clears the input so the next entry can
    /// start fresh. No-op for draft panes — subtasks attach to
    /// persisted tasks only (R-21 UI design: drafts show a "save
    /// first" hint in place of the list).
    pub(in crate::workspace) fn submit_new_subtask(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(te) = pane.task_edit_content() else {
            return;
        };
        let Some(task_id) = te.task_id.clone() else {
            return;
        };
        let text = te.new_subtask_input.read(cx).value().to_string();
        let input = te.new_subtask_input.clone();
        if text.trim().is_empty() {
            return;
        }
        self.add_subtask(&task_id, text, cx);
        input.update(cx, |inp, cx_state| {
            inp.set_value(String::new(), window, cx_state)
        });
    }

    /// Begin an inline rename of `subtask_id`. Stamps the shared
    /// rename input with the current title and routes focus to it so
    /// the user can edit immediately. Only one rename can be active at
    /// a time (single shared input).
    pub(in crate::workspace) fn enter_rename_subtask(
        &mut self,
        pane_id: PaneId,
        subtask_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(te) = pane.task_edit_content() else {
            return;
        };
        let Some(task_id) = te.task_id.clone() else {
            return;
        };
        let input_entity = te.editing_subtask_input.clone();
        let title = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .get(&task_id)
            .and_then(|t| t.subtasks.iter().find(|s| s.id == subtask_id).cloned())
            .map(|s| s.title)
            .unwrap_or_default();
        input_entity.update(cx, |inp, cx_state| inp.set_value(title, window, cx_state));
        if let Some(te) = self.task_edit_content_mut_for(pane_id) {
            te.editing_subtask = Some(subtask_id);
        }
        let handle = input_entity.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Commit the inline rename — flushes the input's text into
    /// `rename_subtask` and clears the editing state. Empty / unchanged
    /// titles are dropped by `rename_subtask` itself.
    pub(in crate::workspace) fn commit_rename_subtask(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(te) = pane.task_edit_content() else {
            return;
        };
        let Some(task_id) = te.task_id.clone() else {
            return;
        };
        let Some(subtask_id) = te.editing_subtask.clone() else {
            return;
        };
        let new_title = te.editing_subtask_input.read(cx).text().to_string();
        self.rename_subtask(&task_id, &subtask_id, new_title, cx);
        if let Some(te) = self.task_edit_content_mut_for(pane_id) {
            te.editing_subtask = None;
        }
        cx.notify();
    }

    /// Cancel the inline rename without touching the underlying
    /// subtask. Reached from the TaskEdit pane's outer Esc handler —
    /// `gpui_component::Input` doesn't emit a Cancel event of its own,
    /// so Escape routing lives one level up in `task_edit_pane::render`.
    pub(in crate::workspace) fn cancel_rename_subtask(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        if let Some(te) = self.task_edit_content_mut_for(pane_id) {
            te.editing_subtask = None;
        }
        cx.notify();
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
    /// (R-25) where one wrapping prompt covers multiple panes and the
    /// caller drives the close pass separately. Returns the resolved
    /// `task_id` on success, `None` when the form is invalid (the
    /// caller should keep the pane open in that case).
    pub(in crate::workspace) fn commit_task_edit_pane(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> Option<TaskId> {
        let form = self.read_task_edit_form(pane_id, cx)?;
        if matches!(form.branch_validation, BranchValidation::Invalid { .. }) {
            return None;
        }

        // Empty sentinel → `None`; non-empty → registered worktree
        // path. The path is round-tripped as a string through the
        // `SelectState` value, which means we can't statically
        // distinguish "not in the worktree list anymore" from "user
        // picked a stale option" — but `start_task` re-runs
        // `branch_for_worktree_path` and falls back to git's default
        // when the lookup misses, so the worst case is the same
        // behaviour as `None` (R-19 risk note).
        let base_path: Option<std::path::PathBuf> = if form.base_value.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(&form.base_value))
        };

        let task_id = match &form.task_id {
            Some(id) => {
                self.update_task(
                    id,
                    form.title.clone(),
                    form.prompt.clone(),
                    form.notes.clone(),
                    form.auto_execute,
                    base_path.clone(),
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
                if !form.branch.is_empty() {
                    task.branch_name = form.branch.clone();
                }
                task.notes = form.notes.clone();
                task.auto_execute = form.auto_execute;
                let new_id = task.id.clone();
                cx.update_global::<crate::agent::tasks_global::GlobalTasks, _>(|g, _| {
                    g.add(task);
                });
                self.save_tasks_dirty(cx);
                new_id
            }
        };

        // Re-baseline the dirty snapshot so the pane no longer reads
        // as dirty after a successful save (R-25 / I-8).
        if let Some(te) = self.task_edit_content_mut_for(pane_id) {
            te.task_id = Some(task_id.clone());
            te.saved_snapshot = super::pane::TaskEditSnapshot {
                title: form.title.clone(),
                branch: form.branch.clone(),
                prompt: normalize_newlines(&form.prompt),
                notes: normalize_newlines(&form.notes),
                auto_execute: form.auto_execute,
                base_value: form.base_value.clone(),
            };
        }

        Some(task_id)
    }

    /// Close the TaskEdit pane without saving. R-25's full dirty-prompt
    /// flow lives on `close_pane_by_id`; this is the explicit Discard
    /// path the form footer dispatches to.
    pub(in crate::workspace) fn discard_task_edit_pane(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Mark the pane as non-dirty so the close prompt (R-25)
        // doesn't second-guess the user's explicit Discard. Routes
        // through `current_snapshot` so any new field on
        // `TaskEditSnapshot` (e.g. C-1 `base_value`) is captured by
        // a single source of truth rather than duplicated here.
        if let Some(pane) = self.panes.iter().find(|p| p.id == pane_id)
            && let Some(te) = pane.task_edit_content()
        {
            let snapshot = te.current_snapshot(cx);
            if let Some(te) = self.task_edit_content_mut_for(pane_id) {
                te.saved_snapshot = snapshot;
            }
        }
        self.close_pane_by_id(pane_id, window, cx);
    }

    /// Read the current form values without holding a `&mut self`
    /// borrow on `self.panes` past the snapshot.
    fn read_task_edit_form(
        &self,
        pane_id: PaneId,
        cx: &Context<Self>,
    ) -> Option<TaskEditFormSnapshot> {
        let te = self
            .panes
            .iter()
            .find(|p| p.id == pane_id)?
            .task_edit_content()?;
        Some(TaskEditFormSnapshot {
            task_id: te.task_id.clone(),
            title: te.title_input.read(cx).text().to_string(),
            branch: te.branch_input.read(cx).text().to_string(),
            prompt: te.prompt_state.read(cx).text().to_string(),
            notes: te.notes_state.read(cx).text().to_string(),
            auto_execute: te.auto_execute,
            branch_validation: te.branch_validation.clone(),
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
/// path doesn't keep a borrow on `self.panes` past the read step.
struct TaskEditFormSnapshot {
    task_id: Option<TaskId>,
    title: String,
    branch: String,
    prompt: String,
    notes: String,
    auto_execute: bool,
    branch_validation: BranchValidation,
    /// Selected `base_select` value — empty string sentinel for "use
    /// active worktree", otherwise an absolute path string.
    base_value: String,
}

/// CRLF → LF for dirty-comparison snapshots. External editors (vim,
/// VS Code on Windows) may rewrite the prompt file with CRLF; we
/// don't want that to register as a user edit (R-25 risk note).
pub(in crate::workspace) fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// Resolve the on-disk prompt file path for `task` — only meaningful
/// once the task has been started (i.e. has a worktree). Returns
/// `None` for Backlog / drafts.
fn prompt_file_path_for(task: &Task) -> Option<std::path::PathBuf> {
    let wt = task.state.worktree_path()?;
    Some(
        wt.join(".daruda")
            .join(format!("task-{}.md", task.branch_name)),
    )
}

/// Install the watcher + pump for `task`'s prompt file. Returns a
/// `(path, handle, pump)` triple so the builder can stash all three
/// on `TaskEditContent`. All three are `None` when the task isn't in
/// a state that has a prompt file on disk.
fn install_prompt_watcher(
    initial: Option<&Task>,
    pane_id: PaneId,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> (
    Option<super::prompt_watcher::PromptFileWatcherHandle>,
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

    let (events_rx, handle) = super::prompt_watcher::spawn(path.clone());
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
    /// rewrites `<wt>/.daruda/task-<branch>.md`. Reloads the editor
    /// silently when the pane is clean; surfaces a conflict prompt
    /// (Use disk version / Keep my version / Diff) when the pane is
    /// dirty (R-20 / I-13).
    pub(in crate::workspace) fn handle_prompt_file_changed(
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
                    ErrorReport::new("Prompt watcher read failed")
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

        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(te) = pane.task_edit_content() else {
            return;
        };
        let prompt_entity = te.prompt_state.clone();
        let title = pane.title();
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
            crate::surface::strings::PROMPT_WATCHER_HEADING_SUFFIX,
        );
        let receiver = window.prompt(
            gpui::PromptLevel::Warning,
            &heading,
            Some(crate::surface::strings::PROMPT_WATCHER_DETAIL),
            &[
                crate::surface::strings::PROMPT_WATCHER_USE_DISK,
                crate::surface::strings::PROMPT_WATCHER_KEEP_MINE,
                crate::surface::strings::PROMPT_WATCHER_DIFF,
            ],
            cx,
        );

        let path_for_diff = path.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| match answer {
                0 => {
                    let Some(pane) = this.panes.iter().find(|p| p.id == pane_id) else {
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
                    // (R-20 follow-up — replaces the previous "new tab"
                    // fallback.)
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

    /// Open `<wt>/.daruda/task-<branch>.md` in a fresh file viewer
    /// tab (R-20 `[📄 Open file]` button). No-op for tasks that
    /// haven't been started yet — Backlog tasks have no worktree
    /// path, and Started tasks whose prompt file disappeared (e.g.
    /// manual delete) silently bail rather than open a viewer onto a
    /// non-existent file. The button itself is disabled in those
    /// states so this is defensive only.
    pub(in crate::workspace) fn open_task_prompt_file(
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
            let report = ErrorReport::new("Prompt file not found")
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("path", redact_home(&path))
                .dedup("tasks.open_prompt_file.missing")
                .build();
            self.report_error(report, cx);
            return;
        }
        let Some(wt_id) = self
            .worktrees
            .iter()
            .find(|w| path.starts_with(&w.path))
            .map(|w| w.id)
        else {
            let report = ErrorReport::new("Prompt file is outside all known worktrees")
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("path", redact_home(&path))
                .dedup("tasks.open_prompt_file.no_worktree")
                .build();
            self.report_error(report, cx);
            return;
        };
        self.open_files_entry(wt_id, path, window, cx);
    }

    /// Helper used by the R-20 conflict prompt's `[Diff]` branch.
    /// Opens `path` in a file viewer pane *split to the right of* the
    /// owning TaskEdit pane so the user sees the in-pane editor on
    /// the left and the disk version on the right simultaneously
    /// The two-pane layout lets the user compare in-pane edits against
    /// the on-disk version side-by-side. Falls back silently when the
    /// path isn't inside any known worktree.
    fn open_disk_file_for_diff(
        &mut self,
        pane_id: PaneId,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(wt) = self
            .worktrees
            .iter()
            .find(|w| path.starts_with(&w.path))
            .map(|w| w.id)
        else {
            return;
        };
        self.open_file_split_right(wt, path, pane_id, window, cx);
    }
}

/// Build the `base_select` option list from the workspace's current
/// worktrees. The leading empty-string option is the "no explicit
/// base — defer to the active worktree at `start_task` time"
/// sentinel; remaining entries are keyed by absolute path so
/// `commit_task_edit_pane` can round-trip the user's pick back into
/// `Task::base_worktree_path: Option<PathBuf>`.
fn base_worktree_options(ws: &Workspace) -> Vec<SelectOption> {
    let mut options = Vec::with_capacity(ws.worktrees.len() + 1);
    options.push(SelectOption::new(
        "",
        crate::surface::strings::TASK_EDIT_BASE_ACTIVE_LABEL,
    ));
    for w in &ws.worktrees {
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
