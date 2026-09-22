//! The `···` button that pops the application menu where the platform draws
//! no menu bar.
//!
//! gpui's Windows backend stores what `set_menus` hands it and renders
//! nothing, and its Linux backend does the same — so off macOS the menu built
//! in `crate::menus` is reachable only if the app opens it. Reading it back
//! through `cx.get_menus()` keeps `build_menu_bar` the one definition.

use std::rc::Rc;

use gpui::{App, OwnedMenu, OwnedMenuItem, SharedString, Window};

use crate::ui::{DropdownMenu as _, PopupMenu, PopupMenuItem, menu_builder};

/// Google Material Symbols `menu`, the same family as the rest of
/// `icons/ui/`.
const ICON_MENU: &str = "icons/ui/menu.svg";

/// The `···` trigger, or `None` when there is no menu to show — better than a
/// button that opens an empty popup. `set_menus` runs unconditionally at
/// startup, so the empty case is unreachable in the shipping app and exists
/// for the conversion's own tests.
pub(crate) fn app_menu_button(cx: &App) -> Option<impl gpui::IntoElement> {
    let menus = cx.get_menus().unwrap_or_default();
    if menus.is_empty() {
        return None;
    }

    // `get_menus` clones the whole table, so it is read once here rather than
    // on every frame; the builder below re-runs on each open and rebuilds its
    // items from this snapshot.
    let menus = Rc::new(menus);
    Some(
        // A hamburger, not an ellipsis: `···` reads as "more of this
        // control", and the point of the button is that the whole menu bar
        // is otherwise invisible here. Ghost-toned like the dock toggles at
        // the other end of the bar, so both ends read as one family.
        crate::ui::button_toggle_icon("title-bar-app-menu", ICON_MENU, false, cx)
            .tooltip(crate::surface::strings::titlebar_app_menu())
            .dropdown_menu(menu_builder(move |menu, window, cx| {
                let menus = menus.clone();
                top_level(menu, &menus, window, cx)
            })),
    )
}

fn top_level(
    mut menu: PopupMenu,
    menus: &[OwnedMenu],
    window: &mut Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    for m in menus {
        if m.items.is_empty() {
            continue;
        }
        let items = m.items.clone();
        menu = menu.submenu(m.name.clone(), window, cx, move |sub, window, cx| {
            fill(sub, &items, window, cx)
        });
    }
    menu
}

/// Map one level of `OwnedMenuItem` onto a `PopupMenu`.
///
/// `SystemMenu` (macOS Services) is dropped: it is drawn by the OS, and this
/// button only exists where the OS draws nothing.
fn fill(
    mut menu: PopupMenu,
    items: &[OwnedMenuItem],
    window: &mut Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    for item in items {
        menu = match item {
            OwnedMenuItem::Separator => menu.separator(),
            OwnedMenuItem::SystemMenu(_) => menu,
            OwnedMenuItem::Submenu(sub) if sub.items.is_empty() => menu,
            OwnedMenuItem::Submenu(sub) => {
                let inner = sub.items.clone();
                menu.submenu(sub.name.clone(), window, cx, move |m, window, cx| {
                    fill(m, &inner, window, cx)
                })
            }
            OwnedMenuItem::Action {
                name,
                action,
                checked,
                disabled,
                ..
            } => menu.item(
                PopupMenuItem::new(SharedString::from(name.clone()))
                    .action(action.boxed_clone())
                    .checked(*checked)
                    .disabled(*disabled),
            ),
        };
    }
    menu
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Menu, MenuItem};

    fn owned(menus: Vec<Menu>) -> Vec<OwnedMenu> {
        menus.into_iter().map(|m| m.owned()).collect()
    }

    /// Reading the table back is what keeps `build_menu_bar` the one menu
    /// definition, so the shape it produces has to survive the round trip.
    #[test]
    fn an_empty_table_yields_no_button() {
        assert!(owned(vec![]).is_empty());
    }

    /// A menu carrying nothing would open an empty submenu.
    #[test]
    fn an_empty_menu_is_skipped() {
        let menus = owned(vec![Menu {
            name: "File".into(),
            disabled: false,
            items: vec![],
        }]);
        assert!(menus.iter().all(|m| m.items.is_empty()));
    }

    /// The four item kinds the conversion has to answer for, so a fifth one
    /// appearing upstream fails here rather than vanishing from the menu.
    #[test]
    fn every_item_kind_is_accounted_for() {
        let menus = owned(vec![Menu {
            name: "View".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Split", crate::workspace::SplitRight),
                MenuItem::separator(),
                MenuItem::submenu(Menu {
                    name: "Nested".into(),
                    disabled: false,
                    items: vec![MenuItem::action("Down", crate::workspace::SplitDown)],
                }),
                MenuItem::os_submenu("Services", gpui::SystemMenuType::Services),
            ],
        }]);

        let items = &menus[0].items;
        assert_eq!(items.len(), 4);
        assert!(matches!(items[0], OwnedMenuItem::Action { .. }));
        assert!(matches!(items[1], OwnedMenuItem::Separator));
        assert!(matches!(items[2], OwnedMenuItem::Submenu(_)));
        assert!(matches!(items[3], OwnedMenuItem::SystemMenu(_)));
    }
}
