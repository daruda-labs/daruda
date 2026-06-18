//! Task picker modal — wraps `crate::ui::list` so the command palette's
//! Start/Cancel/Reopen/Retry/Delete Task entries can prompt the user
//! to pick a target task before the action runs.
//!
//! State filtering matches the per-state action surface in
//! [`super::tasks`]:
//!
//! | Action | Eligible task states                       |
//! | ------ | ------------------------------------------ |
//! | Start  | `Backlog`                                  |
//! | Cancel | `Running`                                  |
//! | Reopen | `Done` / `Cancelled` / `Error`             |
//! | Retry  | `Error`                                    |
//! | Delete | every state                                |
//!
//! Cancellation routes through `ListEvent::Cancel` (Escape) and
//! submission through `ListEvent::Confirm(IndexPath)`. Both paths end
//! with `window.close_dialog(cx)`.

use crate::ui::theme;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px,
};

use crate::surface::strings;
use crate::ui::WindowExt as _;
use crate::ui::list::{FilteredItem, FilteredListState, ListEvent, list, searchable_list_state};
use crate::workspace::ModalView;
use crate::workspace::Workspace;
use daruda_terminal::ux::strings as ux_strings;

/// Which action runs after the user picks a task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskPickAction {
    Start,
    Cancel,
    Reopen,
    Retry,
    Delete,
    /// Picks a task to open in a TaskEdit pane — wired from the
    /// Command Palette `edit_task` entry (C-2 review). Any task state
    /// is eligible because Edit is a pure metadata mutation.
    Edit,
}

impl TaskPickAction {
    /// Modal title shown by the dialog. Reads naturally as "Start
    /// Task", "Cancel Task", … so the user knows which dispatch they
    /// are about to commit to.
    pub(in crate::workspace) fn modal_title(self) -> String {
        match self {
            Self::Start => strings::task_picker_title_start(),
            Self::Cancel => strings::task_picker_title_cancel(),
            Self::Reopen => strings::task_picker_title_reopen(),
            Self::Retry => strings::task_picker_title_retry(),
            Self::Delete => strings::task_picker_title_delete(),
            Self::Edit => strings::task_picker_title_edit(),
        }
    }

    /// Whether `state` is eligible for this action. The same filter
    /// also drives empty-state messaging.
    fn applies_to(self, state: &daruda_store::tasks::TaskState) -> bool {
        match self {
            Self::Start => matches!(state, daruda_store::tasks::TaskState::Backlog),
            Self::Cancel => matches!(state, daruda_store::tasks::TaskState::Running { .. }),
            Self::Reopen => matches!(
                state,
                daruda_store::tasks::TaskState::Done { .. }
                    | daruda_store::tasks::TaskState::Cancelled { .. }
                    | daruda_store::tasks::TaskState::Error { .. }
            ),
            Self::Retry => matches!(state, daruda_store::tasks::TaskState::Error { .. }),
            Self::Delete | Self::Edit => true,
        }
    }
}

/// List row — owns the stable task id and a precomputed label so the
/// substring filter doesn't re-build a SharedString every keystroke.
#[derive(Clone)]
pub struct TaskPickItem {
    pub id: String,
    pub label_text: SharedString,
}

impl FilteredItem for TaskPickItem {
    fn label(&self) -> SharedString {
        self.label_text.clone()
    }
}

pub struct TaskPickerModal {
    panel_focus_handle: FocusHandle,
    list_state: Entity<FilteredListState<TaskPickItem>>,
    action: TaskPickAction,
    workspace: WeakEntity<Workspace>,
    _list_sub: Subscription,
    /// Empty-state hint shown when nothing satisfies the action's
    /// state filter. Letting the list render an empty list and the
    /// shell render a hint-only banner keeps the layout consistent
    /// across populated / empty cases.
    empty_hint: Option<SharedString>,
}

impl TaskPickerModal {
    /// Build the list of items from a snapshot of the app-wide
    /// `GlobalTasks`. Called by `Workspace::open_task_picker_modal`
    /// *before* opening the modal, while the workspace borrow is still
    /// in scope — passing the result via [`Self::new`] avoids a
    /// re-entrant read from inside the modal's constructor
    /// (G2 / pitfall §4).
    pub fn build_items(
        state: &daruda_store::tasks::TasksState,
        action: TaskPickAction,
    ) -> Vec<TaskPickItem> {
        let mut tasks: Vec<&daruda_store::tasks::Task> = state
            .tasks
            .iter()
            .filter(|t| action.applies_to(&t.state))
            .collect();
        tasks.sort_by_key(|t| std::cmp::Reverse(t.created_at));
        tasks
            .into_iter()
            .map(|t| TaskPickItem {
                id: t.id.clone(),
                label_text: SharedString::from(format!(
                    "{}  —  {}",
                    t.title,
                    state_label(&t.state),
                )),
            })
            .collect()
    }

    pub fn new(
        workspace: WeakEntity<Workspace>,
        action: TaskPickAction,
        items: Vec<TaskPickItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let empty_hint = if items.is_empty() {
            Some(SharedString::from(empty_message(action)))
        } else {
            None
        };

        let list_state = cx.new(|cx| searchable_list_state(items, window, cx));

        let _list_sub = cx.subscribe_in(
            &list_state,
            window,
            move |this, state, ev: &ListEvent, window, cx| match ev {
                ListEvent::Confirm(ix) => {
                    let id = state.read(cx).delegate().item_at(*ix).map(|i| i.id.clone());
                    this.dispatch(id, window, cx);
                }
                ListEvent::Cancel => this.dismiss(window, cx),
                ListEvent::Select(_) => {}
            },
        );

        Self {
            panel_focus_handle: cx.focus_handle(),
            list_state,
            action,
            workspace,
            _list_sub,
            empty_hint,
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }

    /// Dispatch the workspace method matching `self.action`. Always
    /// closes the dialog — even when the id is missing — so the modal
    /// cannot be left orphaned by a concurrent delete.
    fn dispatch(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let action = self.action;
        window.close_dialog(cx);
        let Some(id) = id else {
            return;
        };
        if let Some(ws) = self.workspace.upgrade() {
            ws.update(cx, |ws, cx| match action {
                TaskPickAction::Start => ws.start_task(&id, window, cx),
                TaskPickAction::Cancel => ws.cancel_task(&id, cx),
                TaskPickAction::Reopen => ws.reopen_task(&id, cx),
                TaskPickAction::Retry => ws.retry_task(&id, window, cx),
                TaskPickAction::Delete => ws.open_delete_task_confirm(&id, window, cx),
                TaskPickAction::Edit => ws.open_task_edit_pane(Some(id), window, cx),
            });
        }
    }
}

/// Compact one-line state summary used as the row's secondary label.
/// Mirrors `right_panel::tasks::state_label` shape (state name only)
/// so the picker reads as the same row, just shorter. The `Done` /
/// `Error` cases reuse the `*_LABEL_PREFIX` consts bare ("Done",
/// "Error") — the picker intentionally drops the suffix the right
/// panel appends (e.g. "Done (Stop)"), so the `_PREFIX` name refers to
/// the panel's usage, not this one.
fn state_label(state: &daruda_store::tasks::TaskState) -> &'static str {
    match state {
        daruda_store::tasks::TaskState::Backlog => ux_strings::RIGHT_PANEL_TASK_BACKLOG_LABEL,
        daruda_store::tasks::TaskState::Running { .. } => {
            ux_strings::RIGHT_PANEL_TASK_RUNNING_LABEL
        }
        daruda_store::tasks::TaskState::Done { .. } => {
            ux_strings::RIGHT_PANEL_TASK_DONE_LABEL_PREFIX
        }
        daruda_store::tasks::TaskState::Error { .. } => {
            ux_strings::RIGHT_PANEL_TASK_ERROR_LABEL_PREFIX
        }
        daruda_store::tasks::TaskState::Cancelled { .. } => {
            ux_strings::RIGHT_PANEL_TASK_CANCELLED_LABEL
        }
    }
}

fn empty_message(action: TaskPickAction) -> String {
    match action {
        TaskPickAction::Start => strings::task_picker_empty_start(),
        TaskPickAction::Cancel => strings::task_picker_empty_cancel(),
        TaskPickAction::Edit => strings::task_picker_empty_edit(),
        TaskPickAction::Reopen => strings::task_picker_empty_reopen(),
        TaskPickAction::Retry => strings::task_picker_empty_retry(),
        TaskPickAction::Delete => strings::task_picker_empty_delete(),
    }
}

impl Focusable for TaskPickerModal {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        // Delegate to the list state so keystrokes land in the query
        // input directly — same pattern as CreateTaskModal landing
        // focus on its title input.
        self.list_state.focus_handle(cx)
    }
}

impl ModalView for TaskPickerModal {}

impl Render for TaskPickerModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if let Some(hint) = self.empty_hint.clone() {
            div()
                .py(px(theme::RIGHT_PANEL_PAD_Y))
                .text_color(theme::current(cx).text_subtle)
                .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                .child(hint)
                .into_any_element()
        } else {
            list(&self.list_state).into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .key_context("TaskPickerModal")
            .track_focus(&self.panel_focus_handle)
            // Tab containment required by the modal tab rule — even
            // though the body is a single `list()` that handles its own
            // arrow-key navigation, the group anchor keeps Tab from
            // escaping the dialog into the workspace behind it.
            .tab_group()
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key.as_str() == "escape" {
                    this.dismiss(window, cx);
                    cx.stop_propagation();
                }
            }))
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body)
    }
}
