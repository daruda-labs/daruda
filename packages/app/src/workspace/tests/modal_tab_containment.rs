//! Tests for modal Tab containment.
//!
//! The vendored `Root` implements it: its Tab / Shift+Tab actions
//! (`advance_focus` in `vendor/gpui-kit/crates/base/src/root.rs`) keep focus
//! inside the active focus trap a dialog installs
//! (`vendor/gpui-kit/crates/base/src/dialog.rs`), and park focus on the
//! trap's own handle when no stop inside it is reachable, so Tab does not
//! oscillate between the dialog chrome and the next global stop.
//!
//! These tests catch the loudest regression class — a panic, a borrow
//! conflict, or an endless walk on the Tab paths with a zero-stop dialog
//! open — and drive both directions so a change that breaks one of them is
//! visible.

use gpui::{AppContext as _, TestAppContext};

use super::build_workspace;
use crate::ui;

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
        crate::ui::dialog::open_confirm_dialog(
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
