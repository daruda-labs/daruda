//! Context menu items for a Project header (left-dock worktrees view,
//! §5.1). Mirrors `group_menu::build_group_menu_items` — flat
//! `Vec<ContextMenuItem>` ready for `Workspace::open_context_menu`.
//!
//! Items: Rename · Move to Group · Delete · Open in New Window.
//!
//! The active-project–scoped action handlers (`on_rename_active_project`,
//! `on_move_active_project_to_group`, global `CloseProject`) are reused
//! by first snapping the workspace focus to the right-clicked project
//! (matches the §5.5 `project header click → last_active_worktree_id`
//! semantics — the menu just makes the snap explicit). "Open in New
//! Window" routes straight to `Workspace::open_project_in_new_window`
//! without snapping the focus — the user explicitly asked for a
//! separate window, so the current workspace's active state stays
//! untouched.

use daruda_store::project::{ProjectId, WorktreeId, WorktreeRef};
use gpui::WeakEntity;

use crate::surface::strings as s;
use crate::ui::ContextMenuItem;
use crate::workspace::{MoveActiveProjectToGroup, RenameActiveProject, Workspace};

pub(in crate::workspace) fn build_project_menu_items(
    project_id: ProjectId,
    last_active_worktree_id: WorktreeId,
    ws: WeakEntity<Workspace>,
) -> Vec<ContextMenuItem> {
    let mut items: Vec<ContextMenuItem> = Vec::new();

    let snap_target = WorktreeRef {
        project: project_id,
        worktree: last_active_worktree_id,
    };

    // -- Rename --
    let ws_rename = ws.clone();
    items.push(ContextMenuItem::new(
        s::PROJECT_MENU_RENAME,
        move |_, window, app_cx| {
            let Some(workspace) = ws_rename.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.activate_worktree(snap_target, window, cx);
                ws.on_rename_active_project(&RenameActiveProject, window, cx);
            });
        },
    ));

    // -- Move to Group --
    let ws_move = ws.clone();
    items.push(ContextMenuItem::new(
        s::PROJECT_MENU_MOVE_TO_GROUP,
        move |_, window, app_cx| {
            let Some(workspace) = ws_move.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.activate_worktree(snap_target, window, cx);
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
    // The on-submit branch defers `close_active_project` via
    // `app_cx.defer`. The dialog `submit` calls
    // `window.close_dialog(cx)` immediately before invoking
    // on_submit, so running `close_active_project` synchronously
    // would mutate `main_area.tabs` while the modal entity is still
    // mid-teardown — the workspace re-renders with the tab swap only
    // partially observable and the center pane comes back empty.
    // `app_cx.defer` postpones until the current event cycle drains,
    // by which point the modal is fully gone and the workspace
    // update sees a clean slate.
    let ws_delete = ws.clone();
    items.push(ContextMenuItem::new(
        s::PROJECT_MENU_DELETE,
        move |_, window, app_cx| {
            let Some(workspace) = ws_delete.upgrade() else {
                return;
            };
            let window_handle = window.window_handle();
            let ws_for_submit = ws_delete.clone();
            workspace.update(app_cx, |ws, cx| {
                ws.activate_worktree(snap_target, window, cx);
                let Some(project_name) = ws.active_project_name() else {
                    return;
                };
                crate::workspace::delete_project_modal::open_delete_project_modal(
                    project_name,
                    move |choice, _window, app_cx| {
                        use crate::workspace::delete_project_modal::DeleteProjectChoice;
                        let ws_weak = ws_for_submit.clone();
                        app_cx.defer(move |app_cx| {
                            let Some(ws) = ws_weak.upgrade() else {
                                return;
                            };
                            crate::windows::try_update_workspace_window(
                                window_handle,
                                app_cx,
                                "project_menu.delete",
                                move |window, cx_w| match choice {
                                    DeleteProjectChoice::KeepOnDisk => {
                                        let keep = ws.update(cx_w, |ws, cx| {
                                            ws.close_active_project(window, cx)
                                        });
                                        if !keep {
                                            window.remove_window();
                                            crate::windows::ensure_welcome_if_last(cx_w);
                                        }
                                    }
                                    DeleteProjectChoice::DeleteOnDisk => {
                                        ws.update(cx_w, |ws, cx| {
                                            ws.delete_active_project_on_disk(window, cx);
                                        });
                                    }
                                },
                            );
                        });
                    },
                    window,
                    cx,
                );
            });
        },
    ));

    items.push(ContextMenuItem::separator());

    // -- Open in New Window --
    let ws_open = ws.clone();
    items.push(ContextMenuItem::new(
        s::PROJECT_MENU_OPEN_IN_NEW_WINDOW,
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
