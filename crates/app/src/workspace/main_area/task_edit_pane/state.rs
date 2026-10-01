//! The TaskEdit form's state — the entities a pane holds, the values Save
//! reads off them, and the checks that gate Save.

use daruda_store::tasks::{TaskAgentSurface, TaskId};
use gpui::{App, Entity, FocusHandle, ScrollHandle, SharedString, Subscription, Task};

/// Markdown-form editor pane for a single Task. Lives at the same
/// `PaneLayout::Pane` level as Terminal and File so users can split a
/// TaskEdit alongside a running shell. `task_id = None` means this is a
/// draft: nothing is persisted to `tasks.json` until the user presses
/// `[Save Draft]` or `[Start]`, and the layout serializer skips drafts
/// so they don't survive a session restart.
pub(in crate::workspace) struct TaskEditContent {
    pub(in crate::workspace) task_id: Option<TaskId>,
    pub(in crate::workspace) title_input: Entity<crate::ui::InputState>,
    pub(in crate::workspace) branch_input: Entity<crate::ui::InputState>,
    pub(in crate::workspace::main_area) draft_subtasks: Vec<daruda_store::tasks::SubTask>,
    pub(in crate::workspace::main_area) preview_prompt: bool,
    pub(in crate::workspace::main_area) settings_open: bool,
    pub(in crate::workspace::main_area) notes_open: bool,
    pub(in crate::workspace::main_area) branch_validation: BranchValidation,
    /// Dropdown mapping lane picks to `Task::base_worktree_path`. The
    /// empty-string sentinel means "no explicit base — branch from the
    /// active lane at run time"; every other value is the absolute path of a
    /// registered lane. Sits in the focus chain between prompt and notes.
    pub(in crate::workspace) base_select: Entity<crate::ui::select::SelectState>,
    pub(in crate::workspace::main_area) run_in: RunInChoice,
    /// Registered lanes keyed by absolute path; read under
    /// `RunInChoice::ExistingLane` only.
    pub(in crate::workspace) lane_select: Entity<crate::ui::select::SelectState>,
    /// Prompt editor state (`code_editor("markdown")` for line numbers +
    /// syntax highlight), shared with the renderer via
    /// `crate::ui::markdown_editor(&state)`.
    pub(in crate::workspace) prompt_state: Entity<gpui_component::input::InputState>,
    pub(in crate::workspace) notes_state: Entity<gpui_component::input::InputState>,
    pub(in crate::workspace::main_area) auto_execute: bool,
    /// Execution surface the task will run on when started — mirrors
    /// `Task::agent_surface`. Terminal CLI (default) or in-app Agent
    /// chat (ACP). Flipped in-place by the form's surface selector, the
    /// same plain-data pattern as `auto_execute`.
    pub(in crate::workspace::main_area) agent_surface: TaskAgentSurface,
    pub(in crate::workspace::main_area) focus_handle: FocusHandle,
    pub(in crate::workspace::main_area) cached_title: SharedString,
    /// Baseline snapshot for dirty comparison. Reset to
    /// `current_snapshot()` after every successful save.
    pub(in crate::workspace::main_area) saved_snapshot: TaskEditValues,
    pub(in crate::workspace::main_area) _subscriptions: Vec<Subscription>,
    /// FS watcher on `<lane>/.daruda/task-<id>.md`. `None`
    /// when the task is still in `Backlog` (no lane yet) or the
    /// file didn't exist at pane-open time. Dropped with the pane —
    /// `PromptFileWatcherHandle` shuts down the underlying threads.
    pub(in crate::workspace::main_area) _prompt_watcher:
        Option<crate::workspace::main_area::prompt_watcher::PromptFileWatcherHandle>,
    /// GPUI-side pump that polls the watcher's debounced channel and
    /// dispatches `handle_prompt_file_changed`. Dropped with
    /// the pane.
    pub(in crate::workspace::main_area) _prompt_pump: Option<Task<()>>,
    /// Trailing `[+ Add subtask…]` row input. `Submit` (Enter)
    /// dispatches `Workspace::add_subtask` and clears the buffer for
    /// the next entry; the input stays focused so the user can chain
    /// additions.
    pub(in crate::workspace) new_subtask_input: Entity<crate::ui::InputState>,
    /// `Some(subtask_id)` while that row is in inline-rename mode (Enter /
    /// blur commits, Escape cancels). One shared rename input is reused
    /// across rows to avoid IME composition-state churn when switching rows.
    pub(in crate::workspace::main_area) editing_subtask: Option<String>,
    pub(in crate::workspace::main_area) editing_subtask_input: Entity<crate::ui::InputState>,
    /// Scroll handle for the form-body absolute scroll container.
    /// `vertical_scrollbar(&handle)` on the relative parent renders
    /// the visible thumb; `track_scroll(&handle)` on the scroll
    /// container hooks up cursor + wheel + scrollbar drag together.
    pub(in crate::workspace::main_area) body_scroll_handle: ScrollHandle,
}

/// Result of running `validate_branch` over the current branch-input
/// text. Drives the disabled state of `[Save Draft]` / `[Start]` and
/// the inline red-border + reason label under the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace::main_area) enum BranchValidation {
    /// Empty input → a draft gets its default `task-<id>` branch on Save;
    /// a saved task keeps its own.
    Empty,
    /// Passes git ref-name rules.
    Valid,
    /// Fails one of the git ref-name rules. The `reason` is the
    /// short human-readable cause displayed under the field.
    Invalid { reason: SharedString },
    /// A registered lane already checks this branch out, so
    /// `git worktree add -b` would refuse it at Start.
    Exists,
}

/// Where the TaskEdit form will run the task. The existing lane itself is
/// the value of `TaskEditContent::lane_select`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) enum RunInChoice {
    #[default]
    NewWorktree,
    ExistingLane,
}

impl BranchValidation {
    /// Whether the field is currently in an unrecoverable invalid
    /// state — i.e. `Save Draft` / `Start` must stay disabled and the
    /// branch input should render with a red border. `Empty` is *not*
    /// invalid: Save falls back to the task's default or current branch.
    pub(in crate::workspace) fn is_invalid(&self) -> bool {
        matches!(
            self,
            BranchValidation::Invalid { .. } | BranchValidation::Exists
        )
    }
}

/// Plain-data dirty-comparison baseline for a TaskEdit pane. Lives
/// on `TaskEditContent::saved_snapshot` and is recomputed via
/// `current_snapshot()` on every dirty check / save.
///
/// Holds the text as typed; line endings are normalised only when two
/// snapshots are compared, so a CRLF disk reload doesn't read as an edit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct TaskEditValues {
    pub(in crate::workspace::main_area) draft_subtasks: Vec<daruda_store::tasks::SubTask>,
    pub(in crate::workspace::main_area) title: String,
    pub(in crate::workspace::main_area) branch: String,
    pub(in crate::workspace::main_area) prompt: String,
    pub(in crate::workspace::main_area) notes: String,
    pub(in crate::workspace::main_area) auto_execute: bool,
    pub(in crate::workspace::main_area) agent_surface: TaskAgentSurface,
    /// Empty string ↔ `Task::base_worktree_path == None`; non-empty ↔
    /// `Some(PathBuf::from(s))`. Plain `String` (not `Option<String>`)
    /// keeps the dirty-comparison `==` path trivial — the user-facing
    /// sentinel is `""` either way.
    pub(in crate::workspace::main_area) base_value: String,
    pub(in crate::workspace::main_area) run_in: RunInChoice,
    /// `lane_select`'s value, `""` when nothing is picked.
    pub(in crate::workspace::main_area) lane_value: String,
}

/// CRLF → LF normaliser, so comparisons never trip on line-ending
/// differences.
pub(in crate::workspace) fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// `a == b` with every CRLF read as LF, compared without allocating.
fn eq_ignoring_crlf(a: &str, b: &str) -> bool {
    fn chars(s: &str) -> impl Iterator<Item = char> + '_ {
        let mut it = s.chars().peekable();
        std::iter::from_fn(move || {
            let c = it.next()?;
            if c == '\r' && it.peek() == Some(&'\n') {
                it.next()
            } else {
                Some(c)
            }
        })
    }
    chars(a).eq(chars(b))
}

impl TaskEditValues {
    /// Whether the user would see these as different — CRLF and LF alike.
    /// Listing every field makes a new one a compile error until it is
    /// compared here.
    fn differs_from(&self, other: &Self) -> bool {
        let Self {
            draft_subtasks,
            title,
            branch,
            prompt,
            notes,
            auto_execute,
            agent_surface,
            base_value,
            run_in,
            lane_value,
        } = self;
        *draft_subtasks != other.draft_subtasks
            || *title != other.title
            || *branch != other.branch
            || !eq_ignoring_crlf(prompt, &other.prompt)
            || !eq_ignoring_crlf(notes, &other.notes)
            || *auto_execute != other.auto_execute
            || *agent_surface != other.agent_surface
            || *base_value != other.base_value
            || *run_in != other.run_in
            || *lane_value != other.lane_value
    }
}

impl TaskEditContent {
    /// The form's current values, read through the input entities — what
    /// Save persists and what the dirty check compares.
    pub(in crate::workspace) fn current_snapshot(&self, cx: &App) -> TaskEditValues {
        TaskEditValues {
            draft_subtasks: self.draft_subtasks.clone(),
            title: self.title_input.read(cx).text().to_string(),
            branch: self.branch_input.read(cx).text().to_string(),
            prompt: self.prompt_state.read(cx).text().to_string(),
            notes: self.notes_state.read(cx).text().to_string(),
            auto_execute: self.auto_execute,
            agent_surface: self.agent_surface,
            base_value: self
                .base_select
                .read(cx)
                .selected_value()
                .map(|v| v.to_string())
                .unwrap_or_default(),
            run_in: self.run_in,
            lane_value: self.lane_value(cx),
        }
    }

    /// The picked lane's path, `""` when none is picked.
    pub(in crate::workspace) fn lane_value(&self, cx: &App) -> String {
        self.lane_select
            .read(cx)
            .selected_value()
            .map(|v| v.to_string())
            .unwrap_or_default()
    }

    /// True when the current form values differ from the last saved
    /// snapshot. The save / discard paths reset `saved_snapshot` to
    /// the value they wrote, so a successful save clears the flag.
    pub(in crate::workspace) fn is_dirty(&self, cx: &App) -> bool {
        self.current_snapshot(cx).differs_from(&self.saved_snapshot)
    }

    pub(in crate::workspace) fn can_save(&self, cx: &App) -> bool {
        let editable = super::run_in_ops::location_editable(self, cx.global());
        let located = !editable
            || match self.run_in {
                RunInChoice::NewWorktree => !self.branch_validation.is_invalid(),
                RunInChoice::ExistingLane => !self.lane_value(cx).is_empty(),
            };
        !self.title_input.read(cx).value().trim().is_empty() && located
    }
}

#[cfg(test)]
mod tests {
    use super::{BranchValidation, TaskEditValues};

    #[test]
    fn only_rule_breaks_and_taken_branches_block_save() {
        assert!(!BranchValidation::Empty.is_invalid());
        assert!(!BranchValidation::Valid.is_invalid());
        assert!(BranchValidation::Exists.is_invalid());
        assert!(
            BranchValidation::Invalid {
                reason: "bad".into()
            }
            .is_invalid()
        );
    }

    #[test]
    fn snapshots_differing_only_by_line_endings_compare_equal() {
        let lf = TaskEditValues {
            prompt: "a\nb".into(),
            notes: "c\nd".into(),
            ..TaskEditValues::default()
        };
        let crlf = TaskEditValues {
            prompt: "a\r\nb".into(),
            notes: "c\r\nd".into(),
            ..TaskEditValues::default()
        };
        assert!(!lf.differs_from(&crlf));
        let edited = TaskEditValues {
            notes: "c\r\ne".into(),
            ..crlf
        };
        assert!(lf.differs_from(&edited));
    }

    #[test]
    fn crlf_is_ignored_but_a_lone_cr_is_not() {
        use super::eq_ignoring_crlf;
        assert!(eq_ignoring_crlf("a\r\nb\r\n", "a\nb\n"));
        assert!(!eq_ignoring_crlf("a\rb", "a\nb"));
        assert!(!eq_ignoring_crlf("a\r\n", "a"));
    }
}
