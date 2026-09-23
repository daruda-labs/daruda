//! An action that cannot be undone opens a confirm dialog and changes
//! nothing until the user accepts it.

use super::*;

/// Run `request` inside the Root window, then report whether a dialog is up.
fn opens_a_dialog(
    cx: &mut TestAppContext,
    wh: WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsView>,
    request: impl FnOnce(&mut SettingsView, &mut gpui::Window, &mut gpui::Context<SettingsView>),
) -> bool {
    // `update_window`, not `wh.update`: the latter leases the Root entity, and
    // opening a dialog updates Root again. A real click leases only the view.
    let win = win.clone();
    cx.update_window(wh.into(), |_, window, cx| {
        win.update(cx, |view, cx| request(view, window, cx));
        crate::ui::WindowExt::has_active_dialog(window, cx)
    })
    .unwrap()
}

#[gpui::test]
fn removing_a_catalog_agent_waits_for_confirmation(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let before = win.read_with(cx, |w, _| w.agent_catalog.len());
    assert!(before > 0, "the default catalog has an entry to remove");

    assert!(opens_a_dialog(cx, wh, &win, |view, window, cx| {
        view.request_remove_agent_catalog_item(0, window, cx)
    }));
    assert_eq!(win.read_with(cx, |w, _| w.agent_catalog.len()), before);
}

#[gpui::test]
fn removing_a_session_host_waits_for_confirmation(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let win_for_add = win.clone();
    wh.update(cx, |_root, window, cx| {
        win_for_add.update(cx, |view, cx| view.add_session_host_row(window, cx));
    })
    .unwrap();
    let before = win.read_with(cx, |w, _| w.session_host_rows.len());

    assert!(opens_a_dialog(cx, wh, &win, |view, window, cx| {
        view.request_remove_session_host_row(0, window, cx)
    }));
    assert_eq!(win.read_with(cx, |w, _| w.session_host_rows.len()), before);
}

#[gpui::test]
fn removing_the_telegram_token_waits_for_confirmation(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    win.update(cx, |w, _| w.telegram_token_configured = true);

    assert!(opens_a_dialog(cx, wh, &win, |view, window, cx| {
        view.request_clear_telegram_token(window, cx)
    }));
    assert!(win.read_with(cx, |w, _| w.telegram_token_configured));
}

#[gpui::test]
fn unpairing_telegram_waits_for_confirmation(cx: &mut TestAppContext) {
    let mut config = daruda_config::Config::default();
    config.telegram.authorized_chat_id = Some(42);
    let (wh, win) = build_window_with_config(cx, config);

    assert!(opens_a_dialog(cx, wh, &win, |view, window, cx| {
        view.request_unpair_telegram(window, cx)
    }));
    assert_eq!(
        win.read_with(cx, |w, _| w.telegram_authorized_chat_id),
        Some(42)
    );
}
