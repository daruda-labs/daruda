//! Context menu items for a Group header (left-dock worktrees view,
//! §5.1). Mirrors the worktree row's `build_context_menu_items` shape
//! — pure builder, takes a `WeakEntity<Workspace>` plus the captured
//! group id + flags, returns a flat `Vec<ContextMenuItem>` ready for
//! `Workspace::open_context_menu`.
//!
//! Items: Rename · color presets (6 + Clear) · Collapse/Expand toggle ·
//! Delete. Sub-menus are intentionally absent (the upstream `ui`
//! widget is flat-only — see `crates/app/src/ui/context_menu.rs`), so
//! colour choices sit at the menu's top level interleaved with the
//! other actions via Separators.

use daruda_store::project::GroupId;
use gpui::{SharedString, WeakEntity};

use crate::surface::strings as s;
use crate::ui::ContextMenuItem;
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers::open_single_field_dialog;

/// Build the flat menu for a group header.
pub(in crate::workspace) fn build_group_menu_items(
    group_id: GroupId,
    current_name: SharedString,
    is_collapsed: bool,
    ws: WeakEntity<Workspace>,
) -> Vec<ContextMenuItem> {
    let mut items: Vec<ContextMenuItem> = Vec::new();

    // -- Rename --
    let ws_rename = ws.clone();
    let initial = current_name.to_string();
    items.push(ContextMenuItem::new(
        s::GROUP_MENU_RENAME,
        move |_, window, app_cx| {
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
        },
    ));

    items.push(ContextMenuItem::separator());

    // -- Color presets --
    for (label, hex) in COLOR_PRESETS {
        let ws_color = ws.clone();
        items.push(ContextMenuItem::new(*label, move |_, _window, app_cx| {
            let Some(workspace) = ws_color.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.recolor_group(group_id, Some((*hex).to_string()), cx);
            });
        }));
    }

    let ws_clear = ws.clone();
    items.push(ContextMenuItem::new(
        s::GROUP_MENU_COLOR_CLEAR,
        move |_, _window, app_cx| {
            let Some(workspace) = ws_clear.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.recolor_group(group_id, None, cx);
            });
        },
    ));

    items.push(ContextMenuItem::separator());

    // -- Collapse / Expand --
    let collapse_label = if is_collapsed {
        s::GROUP_MENU_EXPAND
    } else {
        s::GROUP_MENU_COLLAPSE
    };
    let ws_collapse = ws.clone();
    items.push(ContextMenuItem::new(
        collapse_label,
        move |_, _window, app_cx| {
            let Some(workspace) = ws_collapse.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.toggle_group_collapse(group_id, cx);
            });
        },
    ));

    items.push(ContextMenuItem::separator());

    // -- Delete --
    // No confirmation modal: `delete_group` demotes member projects to
    // ungrouped (no data loss) — only the visual grouping disappears.
    let ws_delete = ws.clone();
    items.push(ContextMenuItem::new(
        s::GROUP_MENU_DELETE,
        move |_, _window, app_cx| {
            let Some(workspace) = ws_delete.upgrade() else {
                return;
            };
            workspace.update(app_cx, |ws, cx| {
                ws.delete_group(group_id, cx);
            });
        },
    ));

    items
}

const COLOR_PRESETS: &[(&str, &str)] = &[
    (s::GROUP_MENU_COLOR_RED, s::GROUP_PRESET_RED),
    (s::GROUP_MENU_COLOR_ORANGE, s::GROUP_PRESET_ORANGE),
    (s::GROUP_MENU_COLOR_YELLOW, s::GROUP_PRESET_YELLOW),
    (s::GROUP_MENU_COLOR_GREEN, s::GROUP_PRESET_GREEN),
    (s::GROUP_MENU_COLOR_BLUE, s::GROUP_PRESET_BLUE),
    (s::GROUP_MENU_COLOR_PURPLE, s::GROUP_PRESET_PURPLE),
];

#[cfg(test)]
mod tests {
    use super::COLOR_PRESETS;

    /// Sanity check the preset table — the count is referenced indirectly
    /// by the `[+] color presets` UI (six swatch rows + Clear) so an
    /// accidental drop here would silently shrink the menu.
    #[test]
    fn color_presets_table_has_six_entries() {
        assert_eq!(COLOR_PRESETS.len(), 6);
        for (label, hex) in COLOR_PRESETS {
            assert!(!label.is_empty(), "preset label must not be empty");
            assert!(
                hex.starts_with('#') && hex.len() == 7,
                "preset hex must be `#RRGGBB`, got {hex:?}",
            );
        }
    }
}
