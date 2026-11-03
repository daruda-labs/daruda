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

fn stored_question_timeout(cx: &mut TestAppContext) -> u32 {
    cx.read(|cx| {
        crate::store::SettingsStore::global(cx)
            .user()
            .agent
            .question_timeout_secs
    })
}

/// The global answer timeout takes whole seconds, `0` (off) included.
#[gpui::test]
fn the_question_timeout_stores_seconds_and_refuses_text(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    assert_eq!(stored_question_timeout(cx), 0, "off by default");
    for (text, refused, expected) in [("5m", true, 0), ("300", false, 300), ("0", false, 0)] {
        set_input(
            &wh,
            &win,
            cx,
            |w| w.agent_question_timeout_input.clone(),
            text,
        );
        win.update(cx, |w, cx| {
            w.error = None;
            let input = w.agent_question_timeout_input.clone();
            w.persist_text_setting(&input, TextSetting::AgentQuestionTimeoutSecs, cx);
            assert_eq!(w.error.is_some(), refused, "{text:?}");
        });
        assert_eq!(stored_question_timeout(cx), expected, "{text:?}");
    }
}

/// An agent card's timeout field: empty follows the global value, a number
/// (`0` included) is the agent's own, and text is refused.
#[gpui::test]
fn an_agent_card_states_its_own_question_timeout(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let resolved = |cx: &mut TestAppContext| {
        win.read_with(cx, |w, cx| {
            w.validate(cx)
                .ok()
                .map(|cfg| cfg.resolved_agents()[0].question_timeout_secs)
        })
    };
    assert_eq!(resolved(cx), Some(None));
    for (text, expected) in [("0", Some(Some(0))), ("90", Some(Some(90))), ("soon", None)] {
        set_agent_row_input(&wh, &win, cx, 0, |r| r.question_timeout_input.clone(), text);
        assert_eq!(resolved(cx), expected, "{text:?}");
    }
}

/// A hand-edited timeout past the field's range loads clamped, so it cannot
/// make the card refuse and block an unrelated catalog edit.
#[gpui::test]
fn an_out_of_range_agent_timeout_loads_clamped_and_other_edits_save(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[[agents]]\nid = \"far\"\nname = \"Far\"\ncommand = \"far-acp\"\nquestion_timeout_secs = 999999\n",
    )
    .unwrap();
    let config = daruda_config::Config::load_from(&path);
    let (wh, win) = build_window_with_config(cx, config);
    set_agent_row_input(&wh, &win, cx, 0, |r| r.name_input.clone(), "Farther");
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("an unrelated edit saves");
        let agent = &cfg.resolved_agents()[0];
        assert_eq!(agent.name, "Farther");
        assert_eq!(
            agent.question_timeout_secs,
            Some(daruda_config::QUESTION_TIMEOUT_MAX_SECS)
        );
    });
}
