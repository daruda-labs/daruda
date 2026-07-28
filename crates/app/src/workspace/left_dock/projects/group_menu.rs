//! Context menu items for a Group header — a flat `Vec<PopupMenuItem>`
//! attached declaratively via `.context_menu(...)`.
//!
//! Items: Rename · color presets (6 + Clear) · Collapse/Expand · Delete.
//! Sub-menus are absent because the menu is built flat, so colour choices
//! sit at the top level separated by `Separator`s.

use daruda_store::project::GroupId;
use gpui::{SharedString, WeakEntity};

use crate::surface::strings as s;
use crate::ui::PopupMenuItem;
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers::open_single_field_dialog;
use crate::workspace::group_ops::group_color_presets;

/// Build the flat menu for a group header.
pub(in crate::workspace) fn build_group_menu_items(
    group_id: GroupId,
    current_name: SharedString,
    is_collapsed: bool,
    ws: WeakEntity<Workspace>,
) -> Vec<PopupMenuItem> {
    let mut items: Vec<PopupMenuItem> = Vec::new();

    // -- Rename --
    let ws_rename = ws.clone();
    let initial = current_name.to_string();
    items.push(
        PopupMenuItem::new(s::group_menu_rename()).on_click(move |_, window, app_cx| {
            let Some(workspace) = ws_rename.upgrade() else {
                return;
            };
            let weak = ws_rename.clone();
            let initial = initial.clone();
            workspace.update(app_cx, |_, cx| {
                open_single_field_dialog(
                    weak,
                    s::group_rename_dialog_title(),
                    s::group_rename_dialog_placeholder(),
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
        }),
    );

    items.push(PopupMenuItem::separator());

    // -- Color presets --
    for (label, hex) in group_color_presets() {
        let ws_color = ws.clone();
        items.push(
            PopupMenuItem::new(label).on_click(move |_, _window, app_cx| {
                let Some(workspace) = ws_color.upgrade() else {
                    return;
                };
                workspace.update(app_cx, |ws, cx| {
                    ws.recolor_group(group_id, Some(hex.to_string()), cx);
                });
            }),
        );
    }

    let ws_clear = ws.clone();
    items.push(PopupMenuItem::new(s::group_menu_color_clear()).on_click(
        move |_, _window, app_cx| {
            let Some(workspace) = ws_clear.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.recolor_group(group_id, None, cx);
            });
        },
    ));

    items.push(PopupMenuItem::separator());

    // -- Collapse / Expand --
    let collapse_label = if is_collapsed {
        s::group_menu_expand()
    } else {
        s::group_menu_collapse()
    };
    let ws_collapse = ws.clone();
    items.push(
        PopupMenuItem::new(collapse_label).on_click(move |_, _window, app_cx| {
            let Some(workspace) = ws_collapse.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.toggle_group_collapse(group_id, cx);
            });
        }),
    );

    items.push(PopupMenuItem::separator());

    // -- Delete --
    // No confirmation modal: `delete_group` demotes member projects to
    // ungrouped (no data loss) — only the visual grouping disappears.
    let ws_delete = ws.clone();
    items.push(
        PopupMenuItem::new(s::group_menu_delete()).on_click(move |_, _window, app_cx| {
            let Some(workspace) = ws_delete.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.delete_group(group_id, cx);
            });
        }),
    );

    items
}

#[cfg(test)]
mod tests {
    use crate::workspace::group_ops::group_color_presets;

    /// Sanity check the shared preset table — the count is referenced
    /// indirectly by the `[+] color presets` UI so an accidental drop
    /// here would silently shrink the menu.
    #[test]
    fn color_presets_table_is_well_formed() {
        assert!(
            group_color_presets().len() >= 6,
            "expected at least six preset colors, got {}",
            group_color_presets().len()
        );
        for (label, hex) in group_color_presets() {
            assert!(!label.is_empty(), "preset label must not be empty");
            assert!(
                hex.starts_with('#') && hex.len() == 7,
                "preset hex must be `#RRGGBB`, got {hex:?}",
            );
        }
    }
}
