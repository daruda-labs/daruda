//! Context menu items for a Project header — a flat `Vec<PopupMenuItem>`
//! attached declaratively via `.context_menu(...)`.
//!
//! Items: Rename · Move to Group · Delete · Open in New Window. The first
//! three reuse active-project–scoped handlers by first snapping focus to
//! the right-clicked project; "Open in New Window" routes straight to
//! `open_project_in_new_window` without snapping, so the current
//! workspace's active state stays untouched.

use daruda_store::project::{LaneId, LaneRef, ProjectId};
use gpui::WeakEntity;

use crate::surface::strings as s;
use crate::ui::PopupMenuItem;
use crate::workspace::render::ws_popup_menu_item;
use crate::workspace::{MoveActiveProjectToGroup, RenameActiveProject, Workspace};

pub(in crate::workspace) fn build_project_menu_items(
    project_id: ProjectId,
    last_active_lane_id: LaneId,
    ws: WeakEntity<Workspace>,
) -> Vec<PopupMenuItem> {
    let mut items: Vec<PopupMenuItem> = Vec::new();

    let snap_target = LaneRef {
        project: project_id,
        lane: last_active_lane_id,
    };

    // -- Rename --
    items.push(ws_popup_menu_item(
        ws.clone(),
        s::project_menu_rename(),
        false,
        move |ws, window, cx| {
            ws.activate_lane(snap_target, window, cx);
            ws.on_rename_active_project(&RenameActiveProject, window, cx);
        },
    ));

    // -- Move to Group --
    items.push(ws_popup_menu_item(
        ws.clone(),
        s::project_menu_move_to_group(),
        false,
        move |ws, window, cx| {
            ws.activate_lane(snap_target, window, cx);
            ws.on_move_active_project_to_group(&MoveActiveProjectToGroup, window, cx);
        },
    ));

    items.push(PopupMenuItem::separator());

    // -- Delete --
    // Open the chooser modal directly rather than dispatching the global
    // `CloseProject` action: from a context-menu `on_mouse_down` callback,
    // `dispatch_action` falls into the focus-bubble chain and gets swallowed
    // before global handlers fire (the keyboard shortcut still routes through
    // the global handler since it dispatches from a key event). Snap focus to
    // the right-clicked project first so the op targets it.
    items.push(ws_popup_menu_item(
        ws.clone(),
        s::project_menu_delete(),
        false,
        move |ws, window, cx| {
            ws.activate_lane(snap_target, window, cx);
            ws.open_delete_active_project_modal(window, cx);
        },
    ));

    items.push(PopupMenuItem::separator());

    // -- Open in New Window --
    items.push(ws_popup_menu_item(
        ws.clone(),
        s::project_menu_open_in_new_window(),
        false,
        move |ws, _window, cx| {
            ws.open_project_in_new_window(project_id, cx);
        },
    ));

    items
}

#[cfg(test)]
mod tests {
    // `build_project_menu_items` is pure plumbing whose handlers delegate to
    // `Workspace` ops already covered by `workspace::tests::projects`, so a
    // duplicate inline harness would add no coverage.
}
