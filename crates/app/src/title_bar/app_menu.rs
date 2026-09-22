//! The `☰` button that pops the application menu where the platform draws
//! no menu bar.
//!
//! gpui's Windows backend stores what `set_menus` hands it and renders
//! nothing, and its Linux backend does the same — so off macOS the menu built
//! in `crate::menus` is reachable only if the app opens it. The table comes
//! from `crate::menus::MenuSnapshot`, refreshed wherever the menu is set, so
//! `build_menu_bar` stays the one definition.

use std::rc::Rc;

use gpui::{Action, App, OwnedMenu, OwnedMenuItem, SharedString, Window};

use crate::ui::{DropdownMenu as _, PopupMenu, PopupMenuItem, menu_builder};

/// Google Material Symbols `menu`, the same family as the rest of
/// `icons/ui/`.
const ICON_MENU: &str = "icons/ui/menu.svg";

/// One row of the popped menu, resolved from the gpui table before any
/// `PopupMenu` exists. Building a `PopupMenu` needs a `Context<PopupMenu>`,
/// which a unit test cannot produce — this split is what makes the mapping
/// testable at all.
pub(super) enum Row {
    Separator,
    Item {
        name: SharedString,
        action: Box<dyn Action>,
        checked: bool,
        disabled: bool,
    },
    Submenu {
        name: SharedString,
        rows: Vec<Row>,
    },
}

impl Row {
    /// `Box<dyn Action>` is not `Clone`, so the derive is unavailable and the
    /// builder closure — which gpui re-runs on every open — needs this.
    fn duplicate(&self) -> Row {
        match self {
            Row::Separator => Row::Separator,
            Row::Submenu { name, rows } => Row::Submenu {
                name: name.clone(),
                rows: rows.iter().map(Row::duplicate).collect(),
            },
            Row::Item {
                name,
                action,
                checked,
                disabled,
            } => Row::Item {
                name: name.clone(),
                action: action.boxed_clone(),
                checked: *checked,
                disabled: *disabled,
            },
        }
    }
}

/// Map one level of the gpui menu onto rows.
///
/// `SystemMenu` (macOS Services) is dropped: the OS draws it, and this button
/// only exists where the OS draws nothing. A submenu left empty by that drop
/// goes too, rather than offering a dead end.
pub(super) fn plan(items: &[OwnedMenuItem]) -> Vec<Row> {
    let mut rows = Vec::new();
    for item in items {
        match item {
            OwnedMenuItem::Separator => rows.push(Row::Separator),
            OwnedMenuItem::SystemMenu(_) => {}
            OwnedMenuItem::Submenu(sub) => {
                let inner = plan(&sub.items);
                if !inner.is_empty() {
                    rows.push(Row::Submenu {
                        name: sub.name.clone(),
                        rows: inner,
                    });
                }
            }
            OwnedMenuItem::Action {
                name,
                action,
                checked,
                disabled,
                ..
            } => rows.push(Row::Item {
                name: SharedString::from(name.clone()),
                action: action.boxed_clone(),
                checked: *checked,
                disabled: *disabled,
            }),
        }
    }
    rows
}

/// The `☰` trigger, or `None` when there is no menu to show — better than a
/// button that opens an empty popup.
pub(crate) fn app_menu_button(cx: &App) -> Option<impl gpui::IntoElement> {
    let menus = cx.try_global::<crate::menus::MenuSnapshot>()?.0.clone();
    if menus.iter().all(|m| m.items.is_empty()) {
        return None;
    }

    Some(
        // A hamburger, not an ellipsis: `···` reads as "more of this
        // control", and the point of the button is that the whole menu bar
        // is otherwise invisible here. Ghost-toned like the dock toggles at
        // the other end of the bar, so both ends read as one family.
        crate::ui::button_toggle_icon("title-bar-app-menu", ICON_MENU, false, cx)
            .tooltip(crate::surface::strings::titlebar_app_menu())
            .dropdown_menu(menu_builder(move |menu, window, cx| {
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
        let rows = plan(&m.items);
        if rows.is_empty() {
            continue;
        }
        let rows = Rc::new(rows);
        menu = menu.submenu(m.name.clone(), window, cx, move |sub, window, cx| {
            fill(sub, &rows, window, cx)
        });
    }
    menu
}

fn fill(
    mut menu: PopupMenu,
    rows: &[Row],
    window: &mut Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    for row in rows {
        menu = match row {
            Row::Separator => menu.separator(),
            Row::Submenu { name, rows } => {
                let inner: Rc<Vec<Row>> = Rc::new(rows.iter().map(Row::duplicate).collect());
                menu.submenu(name.clone(), window, cx, move |m, window, cx| {
                    fill(m, &inner, window, cx)
                })
            }
            Row::Item {
                name,
                action,
                checked,
                disabled,
            } => {
                // The native menu greys out what the focused window cannot
                // answer (`validateMenuItem` on macOS). Without this the
                // Welcome window's popup shows every workspace action live
                // and does nothing when one is picked.
                let unavailable = !cx.is_action_available(action.as_ref());
                menu.item(
                    PopupMenuItem::new(name.clone())
                        .action(action.boxed_clone())
                        .checked(*checked)
                        .disabled(*disabled || unavailable),
                )
            }
        };
    }
    menu
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{SplitDown, SplitRight};
    use gpui::{Menu, MenuItem, SystemMenuType};

    fn plan_of(items: Vec<MenuItem>) -> Vec<Row> {
        let owned = Menu {
            name: "View".into(),
            disabled: false,
            items,
        }
        .owned();
        plan(&owned.items)
    }

    fn shape(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                Row::Separator => "sep".to_owned(),
                Row::Item { name, .. } => format!("item:{name}"),
                Row::Submenu { name, rows } => format!("sub:{name}[{}]", shape(rows).join(",")),
            })
            .collect()
    }

    /// Order is what makes the popped menu recognisably the same menu the
    /// macOS bar shows.
    #[test]
    fn items_keep_their_order() {
        let rows = plan_of(vec![
            MenuItem::action("First", SplitRight),
            MenuItem::separator(),
            MenuItem::action("Second", SplitDown),
        ]);
        assert_eq!(shape(&rows), ["item:First", "sep", "item:Second"]);
    }

    /// The Services entry is drawn by macOS; this button only exists where
    /// nothing draws it, so carrying it through would leave a dead row.
    #[test]
    fn the_system_menu_is_dropped() {
        let rows = plan_of(vec![
            MenuItem::action("Real", SplitRight),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
        ]);
        assert_eq!(shape(&rows), ["item:Real"]);
    }

    /// Dropping the system menu must not leave a submenu that opens onto
    /// nothing.
    #[test]
    fn a_submenu_left_empty_is_dropped() {
        let rows = plan_of(vec![
            MenuItem::submenu(Menu {
                name: "Only services".into(),
                disabled: false,
                items: vec![MenuItem::os_submenu("Services", SystemMenuType::Services)],
            }),
            MenuItem::action("Real", SplitRight),
        ]);
        assert_eq!(shape(&rows), ["item:Real"]);
    }

    /// Nesting has to survive, or the Worktree and Open Recent submenus
    /// flatten into their parent.
    #[test]
    fn nested_submenus_survive() {
        let rows = plan_of(vec![MenuItem::submenu(Menu {
            name: "Outer".into(),
            disabled: false,
            items: vec![MenuItem::submenu(Menu {
                name: "Inner".into(),
                disabled: false,
                items: vec![MenuItem::action("Leaf", SplitDown)],
            })],
        })]);
        assert_eq!(shape(&rows), ["sub:Outer[sub:Inner[item:Leaf]]"]);
    }

    /// A table with nothing to show is what makes `app_menu_button` return
    /// `None` rather than a button opening an empty popup.
    #[test]
    fn a_table_of_only_system_menus_plans_to_nothing() {
        let rows = plan_of(vec![MenuItem::os_submenu(
            "Services",
            SystemMenuType::Services,
        )]);
        assert!(rows.is_empty());
    }

    /// The disabled flag has to reach the row — it is half of what decides
    /// whether the popped item is greyed.
    #[test]
    fn the_disabled_flag_reaches_the_row() {
        let owned = Menu {
            name: "View".into(),
            disabled: false,
            items: vec![MenuItem::action("Off", SplitRight)],
        }
        .owned();
        let mut items = owned.items;
        if let OwnedMenuItem::Action { disabled, .. } = &mut items[0] {
            *disabled = true;
        }
        let rows = plan(&items);
        assert!(matches!(rows[0], Row::Item { disabled: true, .. }));
    }
}
