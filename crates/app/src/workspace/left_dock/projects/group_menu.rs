//! Context menu builder for a Group header (left-dock worktrees view).
//!
//! Returns a closure compatible with [`crate::ui::ContextMenuExt`] —
//! chain `.context_menu(build_group_menu(...))` on the row element.
//! Items: Rename · color presets (6 + Clear) · Collapse/Expand toggle · Delete.

use daruda_store::project::GroupId;
use gpui::{Context, SharedString, WeakEntity, Window};

use crate::surface::strings as s;
use crate::ui::{PopupMenu, PopupMenuItem, menu_builder};
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers::open_single_field_dialog;
use crate::workspace::group_ops::GROUP_COLOR_PRESETS;

pub(in crate::workspace) fn build_group_menu(
    group_id: GroupId,
    current_name: SharedString,
    is_collapsed: bool,
    ws: WeakEntity<Workspace>,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu {
    menu_builder(move |menu, _, _| {
        let ws_rename = ws.clone();
        let initial = current_name.to_string();
        let rename_item =
            PopupMenuItem::new(s::GROUP_MENU_RENAME).on_click(move |_, window, app_cx| {
                let Some(workspace) = ws_rename.upgrade() else {
                    return;
                };
                let weak = ws_rename.clone();
                let initial = initial.clone();
                workspace.update(app_cx, |_, cx| {
                    open_single_field_dialog(
                        weak,
                        s::GROUP_RENAME_DIALOG_TITLE,
                        s::GROUP_RENAME_DIALOG_PLACEHOLDER,
                        Some(&initial),
                        move |ws, value, _window, cx| {
                            let Some(name) = value else {
                                return;
                            };
                            ws.rename_group(group_id, name, cx);
                        },
                        window,
                        cx,
                    );
                });
            });

        let mut menu = menu.item(rename_item).separator();

        for &(label, hex) in GROUP_COLOR_PRESETS {
            let ws_color = ws.clone();
            menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, app_cx| {
                let Some(workspace) = ws_color.upgrade() else {
                    return;
                };
                workspace.update(app_cx, |ws, cx| {
                    ws.recolor_group(group_id, Some(hex.to_string()), cx);
                });
            }));
        }

        let ws_clear = ws.clone();
        let clear_item =
            PopupMenuItem::new(s::GROUP_MENU_COLOR_CLEAR).on_click(move |_, _, app_cx| {
                let Some(workspace) = ws_clear.upgrade() else {
                    return;
                };
                workspace.update(app_cx, |ws, cx| {
                    ws.recolor_group(group_id, None, cx);
                });
            });

        let collapse_label = if is_collapsed {
            s::GROUP_MENU_EXPAND
        } else {
            s::GROUP_MENU_COLLAPSE
        };
        let ws_collapse = ws.clone();
        let collapse_item = PopupMenuItem::new(collapse_label).on_click(move |_, _, app_cx| {
            let Some(workspace) = ws_collapse.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.toggle_group_collapse(group_id, cx);
            });
        });

        // No confirmation: delete_group demotes member projects to ungrouped
        // (no data loss — only the visual grouping disappears).
        let ws_delete = ws.clone();
        let delete_item = PopupMenuItem::new(s::GROUP_MENU_DELETE).on_click(move |_, _, app_cx| {
            let Some(workspace) = ws_delete.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.delete_group(group_id, cx);
            });
        });

        menu.item(clear_item)
            .separator()
            .item(collapse_item)
            .separator()
            .item(delete_item)
    })
}

#[cfg(test)]
mod tests {
    use crate::workspace::group_ops::GROUP_COLOR_PRESETS;

    /// Sanity check the shared preset table — the count is referenced
    /// indirectly by the color presets UI so an accidental drop here
    /// would silently shrink the menu.
    #[test]
    fn color_presets_table_is_well_formed() {
        assert!(
            GROUP_COLOR_PRESETS.len() >= 6,
            "expected at least six preset colors, got {}",
            GROUP_COLOR_PRESETS.len()
        );
        for (label, hex) in GROUP_COLOR_PRESETS {
            assert!(!label.is_empty(), "preset label must not be empty");
            assert!(
                hex.starts_with('#') && hex.len() == 7,
                "preset hex must be `#RRGGBB`, got {hex:?}",
            );
        }
    }
}
