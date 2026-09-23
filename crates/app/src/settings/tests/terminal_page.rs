//! The Terminal page's shell card and its folded Advanced card.

use super::*;

fn stored_shell_program(cx: &mut TestAppContext) -> Option<String> {
    cx.read(|cx| {
        crate::settings_store::SettingsStore::global(cx)
            .user()
            .shell
            .program
            .clone()
    })
}

#[gpui::test]
fn a_shell_program_is_stored_and_an_empty_field_clears_it(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(
        &wh,
        &win,
        cx,
        |w| w.shell_program_input.clone(),
        "/bin/fish",
    );
    win.update(cx, |w, cx| {
        let input = w.shell_program_input.clone();
        w.persist_text_setting(&input, TextSetting::ShellProgram, cx);
    });
    assert_eq!(stored_shell_program(cx).as_deref(), Some("/bin/fish"));

    set_input(&wh, &win, cx, |w| w.shell_program_input.clone(), "   ");
    win.update(cx, |w, cx| {
        let input = w.shell_program_input.clone();
        w.persist_text_setting(&input, TextSetting::ShellProgram, cx);
    });
    assert_eq!(
        stored_shell_program(cx),
        None,
        "blank means the login shell"
    );
}

#[gpui::test]
fn the_advanced_card_starts_closed_and_toggles(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.update(cx, |w, cx| {
        assert!(!w.advanced_open.contains(&BuiltinSection::Terminal));
        w.toggle_advanced(BuiltinSection::Terminal, cx);
        assert!(w.advanced_open.contains(&BuiltinSection::Terminal));
        w.toggle_advanced(BuiltinSection::Terminal, cx);
        assert!(!w.advanced_open.contains(&BuiltinSection::Terminal));
    });
}

#[gpui::test]
fn an_empty_cost_currency_is_refused(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(&wh, &win, cx, |w| w.flow_cost_currency_input.clone(), "  ");
    win.update(cx, |w, cx| {
        let input = w.flow_cost_currency_input.clone();
        w.persist_text_setting(&input, TextSetting::FlowCostCurrency, cx);
        assert!(w.error.is_some(), "a blank currency must not be saved");
    });
    let stored = cx.read(|cx| {
        crate::settings_store::SettingsStore::global(cx)
            .user()
            .flow
            .cost_currency
            .clone()
    });
    assert_eq!(stored, "USD");
}
