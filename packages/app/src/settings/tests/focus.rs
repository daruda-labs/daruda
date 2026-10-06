//! Opening a page puts focus on something the page shows: its first visible
//! input in page order, then a hand-drawn widget, then the panel itself.

use super::*;
use gpui::Focusable as _;

/// Open `section`, then report whether `pick`'s handle holds focus.
fn focus_lands_on(
    cx: &mut TestAppContext,
    wh: WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsView>,
    section: BuiltinSection,
    pick: impl FnOnce(&SettingsView, &gpui::App) -> gpui::FocusHandle,
) -> bool {
    let win = win.clone();
    cx.update_window(wh.into(), |_, window, cx| {
        win.update(cx, |w, cx| w.focus_section(section, window, cx));
        let handle = pick(win.read(cx), cx);
        handle.is_focused(window)
    })
    .unwrap()
}

#[gpui::test]
fn a_page_whose_inputs_are_all_folded_focuses_the_panel(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    assert!(focus_lands_on(
        cx,
        wh,
        &win,
        BuiltinSection::About,
        |w, _| { w.panel_focus_handle.clone() }
    ));
}

#[gpui::test]
fn a_page_focuses_its_first_row_not_its_first_constructed_input(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    assert!(focus_lands_on(
        cx,
        wh,
        &win,
        BuiltinSection::Terminal,
        |w, cx| { w.shell_program_input.read(cx).focus_handle(cx) }
    ));
}

#[gpui::test]
fn a_page_with_only_hand_drawn_inputs_focuses_one_of_them(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    assert!(focus_lands_on(
        cx,
        wh,
        &win,
        BuiltinSection::RemoteControl,
        |w, cx| { w.telegram_token_input.read(cx).focus_handle(cx) }
    ));
}

#[gpui::test]
fn an_opened_advanced_card_is_focusable(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    win.update(cx, |w, cx| w.toggle_advanced(BuiltinSection::About, cx));
    assert!(focus_lands_on(
        cx,
        wh,
        &win,
        BuiltinSection::About,
        |w, cx| { w.logs_retention_input.read(cx).focus_handle(cx) }
    ));
}

#[gpui::test]
fn the_advanced_header_folds_from_the_keyboard(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    vcx.update(|window, cx| {
        win.update(cx, |w, cx| {
            w.focus_section(BuiltinSection::Terminal, window, cx)
        })
    });
    vcx.run_until_parked();
    let header = win.read_with(&vcx, |w, _| {
        w.advanced_focus_handles[&BuiltinSection::Terminal].clone()
    });
    vcx.update(|window, cx| window.focus(&header, cx));
    vcx.run_until_parked();
    let is_open = |vcx: &mut gpui::VisualTestContext| {
        win.read_with(vcx, |w, _| {
            w.advanced_open.contains(&BuiltinSection::Terminal)
        })
    };
    for key in ["enter", "space"] {
        let before = is_open(&mut vcx);
        // GPUI's keystroke helper emits only KeyDown; clicks activate on KeyUp.
        vcx.simulate_keystrokes(key);
        vcx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse(key).unwrap(),
        });
        vcx.run_until_parked();
        assert_ne!(is_open(&mut vcx), before, "{key} toggles the card once");
    }
}
