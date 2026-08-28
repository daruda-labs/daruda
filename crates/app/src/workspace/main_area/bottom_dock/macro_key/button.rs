//! Button widget — click sends `send` (with optional auto-Enter `\r`)
//! to the focused pane's PTY.
//!
//! Delegates visual rendering entirely to `ui::Button::widget()`, keeping
//! only domain-specific logic here: callback wiring and the right-click
//! context menu (Edit / Delete).

use crate::ui::theme;
use daruda_store::panels::{ButtonDisplay, ButtonWidget, TabId};
use gpui::{ClickEvent, IntoElement, SharedString, px};

use super::super::macro_edit_modal::MacroEditModal;
use crate::surface::strings as surface_strings;
use crate::ui::dialog::ButtonVariant;
use crate::ui::{PopupMenuItem, menu_builder};
use crate::workspace::Workspace;
use crate::workspace::layout::BottomDockSnapshot;
use crate::workspace::layout::Dock;
use crate::workspace::render::ws_popup_menu_item;

/// Render one button widget. Returned element is sized for `flex_wrap`
/// — text mode is auto-width, icon mode is fixed-square.
pub(in crate::workspace) fn render(
    tab_id: TabId,
    btn: &ButtonWidget,
    snap: &BottomDockSnapshot,
    cx: &mut gpui::Context<Dock>,
) -> impl IntoElement {
    let widget_id = btn.id.clone();
    let element_id = SharedString::from(format!("widget-button-{}", btn.id));
    let workspace = snap.workspace.clone();

    let on_click = {
        let tab_id = tab_id.clone();
        let widget_id = widget_id.clone();
        let ws = workspace.clone();
        cx.listener(move |_dock, _: &ClickEvent, window, cx| {
            if let Some(w) = ws.upgrade() {
                w.update(cx, |ws, cx| {
                    ws.run_widget(tab_id.clone(), widget_id.clone(), window, cx)
                });
            }
        })
    };

    // Deployed at the workspace root rather than attached declaratively: the
    // bottom dock clips its content, and a menu rendered inside the tile's own
    // subtree would be cut at the dock edge (see `workspace::root_menu`).
    let on_right_click = {
        let tab_id = tab_id.clone();
        let widget_id = widget_id.clone();
        let btn_clone = btn.clone();
        let ws = workspace.clone();
        move |position, window: &mut gpui::Window, cx: &mut gpui::App| {
            let Some(workspace) = ws.upgrade() else {
                return;
            };
            let build = {
                let tab_id = tab_id.clone();
                let widget_id = widget_id.clone();
                let btn_clone = btn_clone.clone();
                let ws = ws.clone();
                menu_builder(move |menu, _window, _cx| {
                    let items = build_widget_context_menu(
                        tab_id.clone(),
                        widget_id.clone(),
                        btn_clone.clone(),
                        ws.clone(),
                    );
                    items.into_iter().fold(menu, |m, item| m.item(item))
                })
            };
            // Build before leasing — `PopupMenu::build` runs the closure
            // synchronously, and a builder that reads the workspace would
            // double-lease-panic inside `update` (CLAUDE.md Pitfall 5).
            let menu = crate::ui::PopupMenu::build(window, cx, move |menu, window, cx| {
                build(menu, window, cx)
            });
            workspace.update(cx, |workspace, cx| {
                workspace.open_context_menu(position, menu, window, cx);
            });
        }
    };

    let mut button = crate::ui::MacroKey::new(element_id, btn.label.clone())
        .on_click(on_click)
        .on_right_click(on_right_click)
        .tooltip(crate::ui::tooltip::text(build_tooltip(btn)));

    button = match btn.display {
        ButtonDisplay::Text => button.fixed_width(px(theme::BUTTON_WIDGET_TILE_WIDTH)),
        ButtonDisplay::Icon => button
            .icon_mode()
            .icon(btn.icon.as_deref().unwrap_or("").to_string()),
    };

    button
}

fn build_tooltip(btn: &ButtonWidget) -> String {
    match btn.shortcut.as_deref() {
        Some(sc) if !sc.is_empty() => format!("{}  ({})", btn.label, sc),
        _ => btn.label.clone(),
    }
}

/// Build the right-click context menu for a button widget.
///
/// Built-in (seeded) buttons omit the Edit entry — their `send` payload
/// is fixed. User-created buttons get both Edit and Delete.
fn build_widget_context_menu(
    tab_id: TabId,
    widget_id: String,
    btn: ButtonWidget,
    workspace: gpui::WeakEntity<Workspace>,
) -> Vec<PopupMenuItem> {
    let mut items = Vec::new();
    if !btn.builtin {
        items.push(edit_item(tab_id.clone(), btn.clone(), workspace.clone()));
    }
    items.push(delete_item(tab_id, widget_id, btn.label, workspace));
    items
}

fn edit_item(
    tab_id: TabId,
    btn: ButtonWidget,
    workspace: gpui::WeakEntity<Workspace>,
) -> PopupMenuItem {
    ws_popup_menu_item(
        workspace,
        surface_strings::ctx_macro_edit(),
        false,
        move |_ws, window, cx| {
            let tab_id = tab_id.clone();
            let btn = btn.clone();
            let workspace_for_modal = cx.entity().downgrade();
            crate::workspace::dialog_helpers::open_form_modal(
                surface_strings::macro_edit_title(),
                None,
                move |window, cx| {
                    MacroEditModal::new(
                        workspace_for_modal.clone(),
                        tab_id.clone(),
                        Some(&btn),
                        window,
                        cx,
                    )
                },
                window,
                cx,
            );
        },
    )
}

fn delete_item(
    tab_id: TabId,
    widget_id: String,
    label: String,
    workspace: gpui::WeakEntity<Workspace>,
) -> PopupMenuItem {
    ws_popup_menu_item(
        workspace,
        surface_strings::ctx_macro_delete(),
        false,
        move |_ws, window, cx| {
            let body = surface_strings::delete_macro_modal_body(&label);
            let workspace_for_modal = cx.entity().downgrade();
            let tab_id = tab_id.clone();
            let widget_id = widget_id.clone();
            crate::workspace::dialog_helpers::open_confirm_dialog(
                surface_strings::delete_macro_modal_title(),
                body,
                surface_strings::delete_macro_confirm_label(),
                ButtonVariant::Danger,
                move |_ev, _window, app_cx| {
                    if let Some(ws) = workspace_for_modal.upgrade() {
                        let tab_id = tab_id.clone();
                        let widget_id = widget_id.clone();
                        ws.update(app_cx, |ws, cx| {
                            ws.delete_widget(tab_id, widget_id, cx);
                        });
                    }
                },
                window,
                cx,
            );
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::panels::ButtonDisplay;

    fn make_btn(label: &str, shortcut: Option<&str>) -> ButtonWidget {
        ButtonWidget {
            id: "w".to_string(),
            label: label.to_string(),
            send: "x".to_string(),
            auto_enter: true,
            display: ButtonDisplay::Text,
            icon: None,
            shortcut: shortcut.map(str::to_string),
            style: None,
            builtin: false,
        }
    }

    #[test]
    fn tooltip_label_only_when_no_shortcut() {
        let btn = make_btn("Claude", None);
        assert_eq!(build_tooltip(&btn), "Claude");
    }

    #[test]
    fn tooltip_includes_shortcut_when_set() {
        let btn = make_btn("Claude", Some("cmd-shift-1"));
        assert_eq!(build_tooltip(&btn), "Claude  (cmd-shift-1)");
    }

    #[test]
    fn tooltip_treats_empty_shortcut_as_none() {
        let btn = make_btn("Claude", Some(""));
        assert_eq!(build_tooltip(&btn), "Claude");
    }
}
