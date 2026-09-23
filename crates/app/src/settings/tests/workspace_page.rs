//! The Workspace page's status-bar switches share one field with the bar's
//! own right-click menu.

use super::*;
use daruda_config::StatusBarItem;

fn hidden(cx: &mut TestAppContext) -> Vec<StatusBarItem> {
    cx.read(|cx| {
        crate::settings_store::SettingsStore::global(cx)
            .user()
            .status_bar
            .hidden_items
            .clone()
    })
}

#[gpui::test]
fn a_status_bar_switch_writes_the_hidden_list(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.update(cx, |w, cx| {
        w.set_status_bar_item_visible(StatusBarItem::Ports, false, cx)
    });
    assert_eq!(hidden(cx), vec![StatusBarItem::Ports]);

    win.update(cx, |w, cx| {
        w.set_status_bar_item_visible(StatusBarItem::Ports, true, cx)
    });
    assert!(hidden(cx).is_empty());
}

/// The bar's menu toggles an item while Settings is open. The next switch
/// in Settings must land on top of that change, not raise a conflict.
#[gpui::test]
fn a_toggle_from_the_bar_menu_is_adopted_before_the_next_switch(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    cx.update(|cx| {
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::ToggleStatusBarItem(
                    StatusBarItem::Flow,
                ))
                .expect("menu toggle");
        });
    });
    cx.run_until_parked();

    win.update(cx, |w, cx| {
        w.set_status_bar_item_visible(StatusBarItem::Ports, false, cx);
        assert!(w.conflict.is_none(), "the menu's change was not adopted");
    });
    let mut now = hidden(cx);
    now.sort_by_key(|i| format!("{i:?}"));
    assert_eq!(now, vec![StatusBarItem::Flow, StatusBarItem::Ports]);
}
