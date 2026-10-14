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
        crate::store::SettingsStore::global(cx)
            .user()
            .flow
            .cost_currency
            .clone()
    });
    assert_eq!(stored, "USD");
}

fn stored_keep_awake_wait(cx: &mut TestAppContext) -> u64 {
    cx.read(|cx| {
        crate::store::SettingsStore::global(cx)
            .user()
            .desktop
            .keep_awake_approval_wait_minutes
    })
}

#[gpui::test]
fn the_approval_wait_limit_rejects_zero_and_text_and_stores_a_positive_value(
    cx: &mut TestAppContext,
) {
    let (wh, win) = build_window(cx);
    for bad in ["0", "soon", "-5", "", "1441"] {
        set_input(
            &wh,
            &win,
            cx,
            |w| w.keep_awake_approval_wait_input.clone(),
            bad,
        );
        win.update(cx, |w, cx| {
            w.error = None;
            let input = w.keep_awake_approval_wait_input.clone();
            w.persist_text_setting(&input, TextSetting::KeepAwakeApprovalWaitMinutes, cx);
            assert!(w.error.is_some(), "{bad:?} must be refused");
        });
        assert_eq!(stored_keep_awake_wait(cx), 30);
    }
    set_input(
        &wh,
        &win,
        cx,
        |w| w.keep_awake_approval_wait_input.clone(),
        "45",
    );
    win.update(cx, |w, cx| {
        let input = w.keep_awake_approval_wait_input.clone();
        w.persist_text_setting(&input, TextSetting::KeepAwakeApprovalWaitMinutes, cx);
    });
    assert_eq!(stored_keep_awake_wait(cx), 45);
}
