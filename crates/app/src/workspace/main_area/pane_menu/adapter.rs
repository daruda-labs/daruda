use std::cell::RefCell;
use std::rc::Rc;

use gpui::{ClipboardItem, Context, Entity, FocusHandle, WeakEntity, Window};

use crate::ui::{PopupMenu, PopupMenuItem};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

use super::sections::compose;
use super::spec::{Activate, MenuEntry, MenuItemSpec};

pub(in crate::workspace) fn pane_context_menu(
    menu: PopupMenu,
    workspace: WeakEntity<Workspace>,
    pane_id: PaneId,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let Some(workspace_entity) = workspace.upgrade() else {
        return menu;
    };
    let Some((entries, action_context)) = workspace_entity.update(cx, |ws, cx| {
        let context = ws.begin_pane_menu(pane_id, None, window, cx)?;
        let action_context = ws.pane_menu_action_context(pane_id, cx);
        Some((compose(&context), action_context))
    }) else {
        return menu;
    };
    build_popup(menu, entries, action_context, workspace, window, cx)
}

pub(super) fn build_popup_menu(
    entries: Vec<MenuEntry>,
    action_context: Option<FocusHandle>,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Entity<PopupMenu> {
    PopupMenu::build(window, cx, move |menu, window, cx| {
        build_popup(menu, entries, action_context, workspace, window, cx)
    })
}

fn build_popup(
    menu: PopupMenu,
    entries: Vec<MenuEntry>,
    action_context: Option<FocusHandle>,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let mut menu = menu.small();
    if let Some(handle) = action_context.clone() {
        menu = menu.action_context(handle);
    }
    entries.into_iter().fold(menu, |menu, entry| {
        append_entry(
            menu,
            entry,
            action_context.clone(),
            workspace.clone(),
            window,
            cx,
        )
    })
}

fn append_entry(
    menu: PopupMenu,
    entry: MenuEntry,
    action_context: Option<FocusHandle>,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    match entry {
        MenuEntry::Separator => menu.separator(),
        MenuEntry::Item(spec) => menu.item(popup_item(spec, workspace)),
        MenuEntry::Submenu { label, entries } => {
            let entries = Rc::new(RefCell::new(Some(entries)));
            let workspace = workspace.clone();
            menu.submenu(label, window, cx, move |submenu, window, cx| {
                let entries = entries.borrow_mut().take().unwrap_or_default();
                build_popup(
                    submenu,
                    entries,
                    action_context.clone(),
                    workspace.clone(),
                    window,
                    cx,
                )
            })
        }
    }
}

fn popup_item(spec: MenuItemSpec, workspace: WeakEntity<Workspace>) -> PopupMenuItem {
    match spec {
        MenuItemSpec::Disabled { label, reason } => {
            let item = PopupMenuItem::new(label).disabled(true);
            match reason {
                Some(reason) => item.tooltip(reason),
                None => item,
            }
        }
        MenuItemSpec::Enabled { label, activate } => match activate {
            Activate::Op(op) => PopupMenuItem::new(label).on_click(move |_, window, app| {
                if let Some(entity) = workspace.upgrade() {
                    entity.update(app, |ws, cx| op(ws, window, cx));
                }
            }),
            Activate::Action(action) => PopupMenuItem::new(label).action(action),
            Activate::Clipboard(text) => {
                PopupMenuItem::new(label).on_click(move |_, _window, app| {
                    app.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                })
            }
        },
    }
}
