//! The Agent page's flow cost rows.

use super::*;

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
