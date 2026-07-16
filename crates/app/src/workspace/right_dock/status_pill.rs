//! Status pill — the single per-row dropdown that drives every
//! state-transition and meta action for a Task.
//!
//! The pill shows the task's current state label; clicking it opens a
//! `PopupMenu` of the actions valid for that state, with a `separator`
//! distinguishing transitions from meta actions (Edit / Delete / View
//! error).
//!
//! State → menu matrix:
//!
//! | State     | Items (top → bottom)                                          |
//! |-----------|---------------------------------------------------------------|
//! | Backlog   | Start · Edit · Delete                                         |
//! | Running   | Stop · Open lane · — · Edit · Delete                      |
//! | Done      | Reopen · Open lane · — · Edit · Delete                    |
//! | Error     | Retry · Reopen · Open lane · View error · — · Edit · Delete |
//! | Cancelled | Reopen · Open lane · — · Edit · Delete                    |

use crate::ui::theme;
use daruda_store::tasks::{Task, TaskState};
use gpui::{Hsla, IntoElement, SharedString, Styled as _, px};

use super::super::Workspace;
use super::super::layout::RightDockSnapshot;
use crate::surface::strings;
use crate::ui::Sizable as _;
use crate::ui::{DropdownMenu as _, PopupMenu, PopupMenuItem, button};

/// Build the status-pill trigger + dropdown for a single task row.
///
/// The label is the state label plus `TASK_PILL_CHEVRON`. Height,
/// padding, and inline gap come from the `xsmall()` factory in
/// [`crate::ui::button`]; the pill sets only corner radius and a
/// state-tinted background.
pub(in crate::workspace) fn status_pill(
    task: &Task,
    snap: &RightDockSnapshot,
    state_label: SharedString,
    cx: &gpui::App,
) -> impl IntoElement {
    let task_id = task.id.clone();
    let workspace = snap.workspace.clone();
    let state = task.state.clone();

    let pill_id = SharedString::from(format!("task-pill-{}", task.id));
    let label = SharedString::from(format!("{}{}", state_label, strings::TASK_PILL_CHEVRON));
    let bg = pill_background(&state, cx);

    button(pill_id, label)
        .xsmall()
        .bg(bg)
        .rounded(px(theme::RIGHT_PANEL_STATUS_PILL_RADIUS_PX))
        .dropdown_menu(move |menu, _window, _cx| {
            build_state_menu(&state, &task_id, &workspace, menu)
        })
}

/// State colour tinted with [`theme::RIGHT_PANEL_STATUS_PILL_BG_ALPHA`].
/// The same hue powers the row's indicator, keeping the signal consistent.
fn pill_background(state: &TaskState, cx: &gpui::App) -> Hsla {
    let t = theme::current(cx);
    let base = match state {
        TaskState::Backlog => t.text_subtle,
        TaskState::Running { .. } => t.right_panel_task_running_color,
        TaskState::Done { .. } => t.text_muted,
        TaskState::Error { .. } => theme::ERROR,
        TaskState::Cancelled { .. } => t.text_subtle,
    };
    Hsla {
        a: theme::RIGHT_PANEL_STATUS_PILL_BG_ALPHA,
        ..base
    }
}

fn build_state_menu(
    state: &TaskState,
    task_id: &str,
    workspace: &gpui::WeakEntity<Workspace>,
    menu: PopupMenu,
) -> PopupMenu {
    match state {
        TaskState::Backlog => menu
            .item(start_item(task_id, workspace))
            .item(edit_item(task_id, workspace))
            .item(delete_item(task_id, workspace)),
        TaskState::Running { .. } => menu
            .item(stop_item(task_id, workspace))
            .item(open_item(task_id, workspace))
            .separator()
            .item(edit_item(task_id, workspace))
            .item(delete_item(task_id, workspace)),
        TaskState::Done { .. } => menu
            .item(reopen_item(task_id, workspace))
            .item(open_item(task_id, workspace))
            .separator()
            .item(edit_item(task_id, workspace))
            .item(delete_item(task_id, workspace)),
        TaskState::Error { .. } => menu
            .item(retry_item(task_id, workspace))
            .item(reopen_item(task_id, workspace))
            .item(open_item(task_id, workspace))
            .item(view_error_item(task_id, workspace))
            .separator()
            .item(edit_item(task_id, workspace))
            .item(delete_item(task_id, workspace)),
        TaskState::Cancelled { .. } => menu
            .item(reopen_item(task_id, workspace))
            .item(open_item(task_id, workspace))
            .separator()
            .item(edit_item(task_id, workspace))
            .item(delete_item(task_id, workspace)),
    }
}

// ---------------------------------------------------------------------------
// Menu item factories — one per dispatched Workspace action
// ---------------------------------------------------------------------------

fn start_item(task_id: &str, workspace: &gpui::WeakEntity<Workspace>) -> PopupMenuItem {
    let ws = workspace.clone();
    let id = task_id.to_string();
    PopupMenuItem::new(strings::task_action_start()).on_click(move |_, window, app| {
        if let Some(w) = ws.upgrade() {
            let id = id.clone();
            w.update(app, |this, cx| this.start_task(&id, window, cx));
        }
    })
}

fn stop_item(task_id: &str, workspace: &gpui::WeakEntity<Workspace>) -> PopupMenuItem {
    let ws = workspace.clone();
    let id = task_id.to_string();
    PopupMenuItem::new(strings::task_action_stop()).on_click(move |_, _window, app| {
        if let Some(w) = ws.upgrade() {
            let id = id.clone();
            w.update(app, |this, cx| this.cancel_task(&id, cx));
        }
    })
}

fn open_item(task_id: &str, workspace: &gpui::WeakEntity<Workspace>) -> PopupMenuItem {
    let ws = workspace.clone();
    let id = task_id.to_string();
    PopupMenuItem::new(strings::task_action_open()).on_click(move |_, window, app| {
        if let Some(w) = ws.upgrade() {
            let id = id.clone();
            w.update(app, |this, cx| this.focus_task_lane(&id, window, cx));
        }
    })
}

fn reopen_item(task_id: &str, workspace: &gpui::WeakEntity<Workspace>) -> PopupMenuItem {
    let ws = workspace.clone();
    let id = task_id.to_string();
    PopupMenuItem::new(strings::task_action_reopen()).on_click(move |_, _window, app| {
        if let Some(w) = ws.upgrade() {
            let id = id.clone();
            w.update(app, |this, cx| this.reopen_task(&id, cx));
        }
    })
}

fn retry_item(task_id: &str, workspace: &gpui::WeakEntity<Workspace>) -> PopupMenuItem {
    let ws = workspace.clone();
    let id = task_id.to_string();
    PopupMenuItem::new(strings::task_action_retry()).on_click(move |_, window, app| {
        if let Some(w) = ws.upgrade() {
            let id = id.clone();
            w.update(app, |this, cx| this.retry_task(&id, window, cx));
        }
    })
}

fn view_error_item(task_id: &str, workspace: &gpui::WeakEntity<Workspace>) -> PopupMenuItem {
    let ws = workspace.clone();
    let id = task_id.to_string();
    PopupMenuItem::new(strings::task_action_view_error()).on_click(move |_, window, app| {
        if let Some(w) = ws.upgrade() {
            let id = id.clone();
            w.update(app, |this, cx| {
                this.open_task_error_dialog(&id, window, cx);
            });
        }
    })
}

fn edit_item(task_id: &str, workspace: &gpui::WeakEntity<Workspace>) -> PopupMenuItem {
    let ws = workspace.clone();
    let id = task_id.to_string();
    PopupMenuItem::new(strings::task_action_edit()).on_click(move |_, window, app| {
        if let Some(w) = ws.upgrade() {
            let id = id.clone();
            w.update(app, |this, cx| {
                this.open_task_edit_pane(Some(id), window, cx);
            });
        }
    })
}

fn delete_item(task_id: &str, workspace: &gpui::WeakEntity<Workspace>) -> PopupMenuItem {
    let ws = workspace.clone();
    let id = task_id.to_string();
    PopupMenuItem::new(strings::task_action_delete()).on_click(move |_, window, app| {
        if let Some(w) = ws.upgrade() {
            let id = id.clone();
            w.update(app, |this, cx| {
                this.open_delete_task_confirm(&id, window, cx)
            });
        }
    })
}
