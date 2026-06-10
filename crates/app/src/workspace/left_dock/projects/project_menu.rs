//! Context menu items for a Project header (left-dock lanes view,
//! §5.1). Mirrors `group_menu::build_group_menu_items` — flat
//! `Vec<ContextMenuItem>` ready for `Workspace::open_context_menu`.
//!
//! Items: Rename · Move to Group · Delete · Open in New Window.
//!
//! The active-project–scoped action handlers (`on_rename_active_project`,
//! `on_move_active_project_to_group`, global `CloseProject`) are reused
//! by first snapping the workspace focus to the right-clicked project
//! (matches the §5.5 `project header click → last_active_lane_id`
//! semantics — the menu just makes the snap explicit). "Open in New
//! Window" routes straight to `Workspace::open_project_in_new_window`
//! without snapping the focus — the user explicitly asked for a
//! separate window, so the current workspace's active state stays
//! untouched.

use daruda_store::project::{LaneId, LaneRef, ProjectId};
use gpui::WeakEntity;

use crate::surface::strings as s;
use crate::ui::ContextMenuItem;
use crate::workspace::{MoveActiveProjectToGroup, RenameActiveProject, Workspace};

pub(in crate::workspace) fn build_project_menu_items(
    project_id: ProjectId,
    last_active_lane_id: LaneId,
    ws: WeakEntity<Workspace>,
) -> Vec<ContextMenuItem> {
    let mut items: Vec<ContextMenuItem> = Vec::new();

    let snap_target = LaneRef {
        project: project_id,
        lane: last_active_lane_id,
    };

    // -- Rename --
    let ws_rename = ws.clone();
    items.push(ContextMenuItem::new(
        s::project_menu_rename(),
        move |_, window, app_cx| {
            let Some(workspace) = ws_rename.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.activate_lane(snap_target, window, cx);
                ws.on_rename_active_project(&RenameActiveProject, window, cx);
            });
        },
    ));

    // -- Move to Group --
    let ws_move = ws.clone();
    items.push(ContextMenuItem::new(
        s::project_menu_move_to_group(),
        move |_, window, app_cx| {
            let Some(workspace) = ws_move.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.activate_lane(snap_target, window, cx);
                ws.on_move_active_project_to_group(&MoveActiveProjectToGroup, window, cx);
            });
        },
    ));

    items.push(ContextMenuItem::separator());

    // -- Delete --
    // Open the chooser modal directly instead of dispatching the
    // global `CloseProject` action. From a context-menu
    // `on_mouse_down` callback, `window.dispatch_action(...)` does
    // not reliably reach the registered `cx.on_action(CloseProject)`
    // handler — the action falls into the focus-bubble chain and
    // gets swallowed before global handlers fire, so the user sees
    // the menu close with no modal. The keyboard shortcut path
    // (cmd-shift-W) still routes through the global action handler
    // because the action is dispatched from a key event, where the
    // focus chain is what routes it.
    //
    // The deferred-close dance lives in
    // `open_delete_active_project_modal` (shared with the main-area
    // inaccessible empty-state); the menu just snaps focus to the
    // right-clicked project first so the op targets it (§5.5).
    let ws_delete = ws.clone();
    items.push(ContextMenuItem::new(
        s::project_menu_delete(),
        move |_, window, app_cx| {
            let Some(workspace) = ws_delete.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.activate_lane(snap_target, window, cx);
                ws.open_delete_active_project_modal(window, cx);
            });
        },
    ));

    items.push(ContextMenuItem::separator());

    // -- Open in New Window --
    let ws_open = ws.clone();
    items.push(ContextMenuItem::new(
        s::project_menu_open_in_new_window(),
        move |_, _window, app_cx| {
            let Some(workspace) = ws_open.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.open_project_in_new_window(project_id, cx);
            });
        },
    ));

    items
}

#[cfg(test)]
mod tests {
    // `build_project_menu_items` is pure plumbing: it consumes a
    // `WeakEntity<Workspace>` (only constructible inside a GPUI test
    // context) and returns context-menu items whose handlers delegate to
    // `Workspace::on_rename_active_project` /
    // `on_move_active_project_to_group` / `close_active_project` /
    // `open_project_in_new_window`. Those ops are covered by
    // `workspace::tests::projects` against the same workspace fixtures
    // the menu would dispatch into, so a duplicate inline harness would
    // not add coverage.
}
