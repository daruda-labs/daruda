//! The Workspace page's status-bar switches share one field with the bar's
//! own right-click menu.

use super::*;
use daruda_config::StatusBarItem;

fn hidden(cx: &mut TestAppContext) -> Vec<StatusBarItem> {
    cx.read(|cx| {
        crate::store::SettingsStore::global(cx)
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
        cx.update_global::<crate::store::SettingsStore, _>(|store, _| {
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

/// A pending edit elsewhere on the page stops this window adopting outside
/// changes. The switch still shows the saved list and flips from it.
#[gpui::test]
fn a_pending_edit_does_not_leave_the_switch_behind_the_bar_menu(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(&wh, &win, cx, |w| w.terminal_font_size_input.clone(), "15");
    cx.update(|cx| {
        cx.update_global::<crate::store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::ToggleStatusBarItem(
                    StatusBarItem::Flow,
                ))
                .expect("menu toggle");
        });
    });
    cx.run_until_parked();

    win.update(cx, |w, cx| {
        assert!(
            w.base_config.status_bar.is_visible(StatusBarItem::Flow),
            "the pending edit keeps this window's copy behind"
        );
        w.set_status_bar_item_visible(StatusBarItem::Ports, false, cx);
        assert!(w.conflict.is_none());
    });
    let mut now = hidden(cx);
    now.sort_by_key(|i| format!("{i:?}"));
    assert_eq!(now, vec![StatusBarItem::Flow, StatusBarItem::Ports]);
}

#[gpui::test]
fn dock_size_select_persists_and_reset_restores_normal(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    cx.update_window(wh.into(), |_, window, cx| {
        win.update(cx, |w, cx| {
            let select = w.left_dock_size_select.clone();
            SettingsView::set_select_value(&select, "large", window, cx);
            w.persist_select_setting(&select, SelectSetting::LeftDockSize, cx);
            assert_eq!(
                crate::store::SettingsStore::global(cx)
                    .user()
                    .left_dock
                    .size,
                daruda_config::UiSize::Large
            );
            let target = crate::layout::Target::Select(SelectSetting::LeftDockSize);
            assert!(w.differs_from_default(target));
            w.reset_to_default(target, window, cx);
            assert_eq!(
                crate::store::SettingsStore::global(cx)
                    .user()
                    .left_dock
                    .size,
                daruda_config::UiSize::Normal
            );
            assert_eq!(
                select.read(cx).selected_value().map(|s| s.as_ref()),
                Some("normal")
            );
        });
    })
    .unwrap();
}
