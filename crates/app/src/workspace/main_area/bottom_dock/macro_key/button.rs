//! Button widget — click sends `send` (with optional auto-Enter `\r`)
//! to the focused pane's PTY.
//!
//! Delegates visual rendering entirely to `ui::Button::widget()`, keeping
//! only domain-specific logic here: callback wiring and the right-click
//! context menu (Edit / Delete).

use crate::ui::theme;
use daruda_store::panels::{ButtonDisplay, ButtonWidget, TabId};
use gpui::{App, ClickEvent, IntoElement, MouseDownEvent, Pixels, Point, SharedString, Window, px};

use super::super::macro_edit_modal::MacroEditModal;
use crate::surface::strings as surface_strings;
use crate::ui::ContextMenuItem;
use crate::ui::dialog::ButtonVariant;
use crate::workspace::Workspace;
use crate::workspace::layout::BottomDockSnapshot;
use crate::workspace::layout::Dock;

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

    let on_right_click = {
        let tab_id = tab_id.clone();
        let widget_id = widget_id.clone();
        let btn_clone = btn.clone();
        let ws = workspace.clone();
        cx.listener(move |_dock, ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
            let position: Point<Pixels> = ev.position;
            if let Some(w) = ws.upgrade() {
                let items = build_widget_context_menu(
                    tab_id.clone(),
                    widget_id.clone(),
                    btn_clone.clone(),
                    ws.clone(),
                );
                w.update(cx, |ws, cx| ws.open_context_menu(position, items, cx));
            }
        })
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
) -> Vec<ContextMenuItem> {
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
) -> ContextMenuItem {
    ContextMenuItem::new(
        surface_strings::ctx_macro_edit(),
        move |_ev: &MouseDownEvent, window: &mut Window, app_cx: &mut App| {
            let Some(ws) = workspace.upgrade() else {
                return;
            };
            let tab_id = tab_id.clone();
            let btn = btn.clone();
            let workspace_for_modal = workspace.clone();
            ws.update(app_cx, |ws, cx| {
                ws.close_context_menu(cx);
                crate::workspace::dialog_helpers::open_form_modal(
                    "Edit Macro",
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
            });
        },
    )
}

fn delete_item(
    tab_id: TabId,
    widget_id: String,
    label: String,
    workspace: gpui::WeakEntity<Workspace>,
) -> ContextMenuItem {
    ContextMenuItem::new(
        surface_strings::ctx_macro_delete(),
        move |_ev: &MouseDownEvent, window: &mut Window, app_cx: &mut App| {
            let Some(ws) = workspace.upgrade() else {
                return;
            };
            let body = format!("Delete macro \u{201c}{label}\u{201d}?");
            let workspace_for_modal = workspace.clone();
            let tab_id = tab_id.clone();
            let widget_id = widget_id.clone();
            ws.update(app_cx, |ws, cx| {
                ws.close_context_menu(cx);
            });
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
                app_cx,
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
