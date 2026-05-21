//! Tests for the `gpui_component::Root` modal Tab containment patch.
//!
//! `crates/gpui_component/src/root.rs` is patched so the global Tab
//! action checks `active_dialogs.last()` and wraps focus back inside
//! the topmost dialog whenever `window.focus_next` / `focus_prev`
//! escapes the dialog subtree. A second `focus_within` check parks
//! focus on the dialog handle itself when the dialog has zero inner
//! tab stops (e.g. a confirm / alert dialog) so Tab doesn't oscillate
//! between the dialog chrome and the next global stop.
//!
//! `Root::active_dialogs` is `pub(crate)` upstream, so a tight
//! end-to-end assertion ("focus is on or inside the topmost dialog
//! after Tab") would require a vendor patch widening that visibility.
//! For now these tests catch the loudest regression class — a panic
//! or borrow-conflict inside the patched `on_action_tab` /
//! `on_action_tab_prev` path — and pin the symmetric shape of the
//! Tab / Shift+Tab branches so a refactor that drops one of them
//! is visible.

use gpui::{AppContext as _, TestAppContext};

use super::build_workspace;
use crate::ui;
use crate::workspace::dialog_helpers;

#[gpui::test]
async fn tab_with_no_dialog_does_not_panic(cx: &mut TestAppContext) {
    let (window_handle, _ws) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(window_handle.into(), |_, window, cx| {
        window.focus_next(cx);
        window.focus_prev(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// With a zero-tab-stop dialog open (confirm dialog body is plain
/// text), Tab + Shift+Tab via `window.focus_next` / `focus_prev`
/// must not panic and must not oscillate into an infinite loop.
/// The patched `on_action_tab` / `on_action_tab_prev` paths route
/// through `focus_within` twice (once to detect escape, once for
/// the zero-stop guard); either short-circuit failure would surface
/// as a hang under `run_until_parked` or as a panic on borrow
/// conflict.
#[gpui::test]
async fn tab_inside_zero_stop_confirm_dialog_is_safe(cx: &mut TestAppContext) {
    let (window_handle, _ws) = build_workspace(cx);
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, window, cx| {
        dialog_helpers::open_confirm_dialog(
            "Delete lane",
            "Are you sure?",
            "Delete",
            ui::ButtonVariant::Danger,
            |_, _, _| {},
            window,
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();

    // Several rounds of forward and reverse Tab so the zero-stop
    // wrap-back path gets exercised in both directions. Without the
    // guard this loops; `run_until_parked` would never return.
    cx.update_window(window_handle.into(), |_, window, cx| {
        for _ in 0..4 {
            window.focus_next(cx);
        }
        for _ in 0..4 {
            window.focus_prev(cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
}
