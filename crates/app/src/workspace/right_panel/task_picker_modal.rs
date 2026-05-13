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

use crate::ui::WindowExt as _;
use crate::ui::list::{FilteredItem, FilteredListState, ListEvent, list, searchable_list_state};
use crate::workspace::Workspace;

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
    pub(in crate::workspace) fn modal_title(self) -> &'static str {
        match self {
            Self::Start => "Start Task",
            Self::Cancel => "Cancel Task",
            Self::Reopen => "Reopen Task",
            Self::Retry => "Retry Task",
            Self::Delete => "Delete Task",
            Self::Edit => "Edit Task",
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
/// so the picker reads as the same row, just shorter.
fn state_label(state: &daruda_store::tasks::TaskState) -> &'static str {
    match state {
        daruda_store::tasks::TaskState::Backlog => "Backlog",
        daruda_store::tasks::TaskState::Running { .. } => "Running",
        daruda_store::tasks::TaskState::Done { .. } => "Done",
        daruda_store::tasks::TaskState::Error { .. } => "Error",
        daruda_store::tasks::TaskState::Cancelled { .. } => "Cancelled",
    }
}

fn empty_message(action: TaskPickAction) -> &'static str {
    match action {
        TaskPickAction::Start => "No backlog tasks to start.",
        TaskPickAction::Cancel => "No running tasks to cancel.",
        TaskPickAction::Edit => "No tasks to edit.",
        TaskPickAction::Reopen => "No completed tasks to reopen.",
        TaskPickAction::Retry => "No errored tasks to retry.",
        TaskPickAction::Delete => "No tasks to delete.",
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

impl Render for TaskPickerModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if let Some(hint) = self.empty_hint.clone() {
            div()
                .py(px(theme::RIGHT_PANEL_PAD_Y))
                .text_color(theme::current(cx).dock_placeholder_text)
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
