//! Context menu builder for a Project header (left-dock worktrees view).
//!
//! Returns a closure compatible with [`crate::ui::ContextMenuExt`] —
//! chain `.context_menu(build_project_menu(...))` on the row element.
//! Items: Rename · Move to Group · Delete · Open in New Window.
//!
//! Active-project–scoped action handlers (`on_rename_active_project`,
//! `on_move_active_project_to_group`) are reused by first snapping the
//! workspace focus to the right-clicked project before dispatching.
//! "Open in New Window" routes straight to `open_project_in_new_window`
//! without the snap so the current workspace's active state stays untouched.

use daruda_store::project::{ProjectId, WorktreeId, WorktreeRef};
use gpui::{Context, WeakEntity, Window};

use crate::surface::strings as s;
use crate::ui::{PopupMenu, PopupMenuItem, menu_builder};
use crate::workspace::{MoveActiveProjectToGroup, RenameActiveProject, Workspace};

pub(in crate::workspace) fn build_project_menu(
    project_id: ProjectId,
    last_active_worktree_id: WorktreeId,
    ws: WeakEntity<Workspace>,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu {
    let snap_target = WorktreeRef {
        project: project_id,
        worktree: last_active_worktree_id,
    };

    menu_builder(move |menu, _, _| {
        let ws_rename = ws.clone();
        let rename_item =
            PopupMenuItem::new(s::PROJECT_MENU_RENAME).on_click(move |_, window, app_cx| {
                let Some(workspace) = ws_rename.upgrade() else {
                    return;
                };
                workspace.update(app_cx, |ws, cx| {
                    ws.activate_worktree(snap_target, window, cx);
                    ws.on_rename_active_project(&RenameActiveProject, window, cx);
                });
            });

        let ws_move = ws.clone();
        let move_item =
            PopupMenuItem::new(s::PROJECT_MENU_MOVE_TO_GROUP).on_click(move |_, window, app_cx| {
                let Some(workspace) = ws_move.upgrade() else {
                    return;
                };
                workspace.update(app_cx, |ws, cx| {
                    ws.activate_worktree(snap_target, window, cx);
                    ws.on_move_active_project_to_group(&MoveActiveProjectToGroup, window, cx);
                });
            });

        // Open the delete modal directly instead of dispatching the global
        // `CloseProject` action. From an `on_click` callback,
        // `window.dispatch_action(...)` does not reliably reach the registered
        // `cx.on_action(CloseProject)` handler — the action falls into the
        // focus-bubble chain and gets swallowed before global handlers fire.
        //
        // The on-submit branch defers `close_active_project` via `app_cx.defer`
        // because `window.close_dialog(cx)` runs immediately before invoking
        // on_submit, so running `close_active_project` synchronously would
        // mutate `main_area.tabs` while the modal entity is still mid-teardown.
        let ws_delete = ws.clone();
        let delete_item =
            PopupMenuItem::new(s::PROJECT_MENU_DELETE).on_click(move |_, window, app_cx| {
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
            });

        let ws_open = ws.clone();
        let open_item =
            PopupMenuItem::new(s::PROJECT_MENU_OPEN_IN_NEW_WINDOW).on_click(move |_, _, app_cx| {
                let Some(workspace) = ws_open.upgrade() else {
                    return;
                };
                workspace.update(app_cx, |ws, cx| {
                    ws.open_project_in_new_window(project_id, cx);
                });
            });

        menu.item(rename_item)
            .item(move_item)
            .separator()
            .item(delete_item)
            .separator()
            .item(open_item)
    })
}

#[cfg(test)]
mod tests {
    // `build_project_menu` is pure plumbing: it consumes a
    // `WeakEntity<Workspace>` (only constructible inside a GPUI test
    // context) and returns a PopupMenu builder closure whose handlers
    // delegate to `Workspace::on_rename_active_project` /
    // `on_move_active_project_to_group` / `close_active_project` /
    // `open_project_in_new_window`. Those ops are covered by
    // `workspace::tests::projects` against the same workspace fixtures
    // the menu would dispatch into, so a duplicate inline harness would
    // not add coverage.
}
