//! What the view derives from the transcript besides its items: rows held open
//! by a send, the briefing a prompt carries, titles, and the compact bar split.

use super::*;

#[gpui::test]
fn command_analysis_follows_transcript_rebuilds(cx: &mut gpui::TestAppContext) {
    use daruda_transcript::command_analysis::CommandEffect;
    let handle = make_test_view(cx);
    handle
        .update(cx, |view, _window, _cx| {
            let mut item = tool_call(
                "shell",
                daruda_acp::ToolStatusView::Completed,
                Some("parent"),
            );
            let daruda_acp::ChatItem::ToolCall(call) = &mut item else {
                panic!("tool fixture");
            };
            call.kind = daruda_acp::ToolKindView::Execute;
            call.raw_input = Some(serde_json::json!({ "command": "git status" }));
            view.items = vec![item];
            view.rebuild_rows();
            assert_eq!(
                view.command_analysis.get("shell").unwrap().effect,
                Some(CommandEffect::Read)
            );
            view.items.clear();
            view.rebuild_rows();
            assert!(view.command_analysis.get("shell").is_none());
        })
        .expect("window is live");
}

#[gpui::test]
fn host_is_dark_tracks_agent_chat_background_not_ui_theme(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let light_ui = daruda_ui::theme::DarudaTheme {
            title_bar_bg: gpui::Hsla {
                h: 0.0,
                s: 0.0,
                l: 0.95,
                a: 1.0,
            },
            ..Default::default()
        };
        cx.set_global(light_ui);

        daruda_ui::theme::set_agent_chat_bg(cx, 0, 0, 0);
        assert!(
            crate::view::AgentChatView::host_is_dark(cx),
            "dark terminal-backed chat background should drive dark Mermaid keys"
        );

        daruda_ui::theme::set_agent_chat_bg(cx, 255, 255, 255);
        assert!(
            !crate::view::AgentChatView::host_is_dark(cx),
            "light terminal-backed chat background should drive light Mermaid keys"
        );
    });
}

// What a sent prompt does to the transcript: the briefing, and which response an echo holds open.

/// The contract the orchestrator briefing rests on: the agent is told more
/// than the transcript shows, exactly once.
///
/// If the two ever converged, the person would either read a wall of
/// instructions they never wrote (echo carries it) or the agent would never be
/// briefed at all (wire drops it).
#[gpui::test]
fn a_briefing_rides_the_first_prompt_and_is_never_echoed(cx: &mut gpui::TestAppContext) {
    let handle = make_test_view(cx);
    handle
        .update(cx, |view, _window, cx| {
            view.set_briefing("YOU ARE THE ORCHESTRATOR".to_string());

            let first = view.wire_text_for_test("open a tab on main");
            assert!(
                first.starts_with("YOU ARE THE ORCHESTRATOR"),
                "the agent reads the briefing before the prompt: {first}"
            );
            assert!(
                first.ends_with("open a tab on main"),
                "and the person's words survive it verbatim: {first}"
            );

            // One-shot: a second turn is the person's words and nothing else.
            assert_eq!(view.wire_text_for_test("and another"), "and another");

            // The transcript is the echo, and the echo never saw a briefing.
            view.echo_prompt("open a tab on main".to_string(), cx);
            let shown: Vec<&str> = view
                .items
                .iter()
                .filter_map(|i| match i {
                    daruda_acp::ChatItem::UserText(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(shown, vec!["open a tab on main"]);
        })
        .expect("window is live");
}

/// A send holds open the response the user was reading, and only that one:
/// the next send releases it, a user collapse outranks the hold, and the holds
/// never accumulate into overrides — picking a Mode still governs every
/// response after several sends.
#[gpui::test]
fn a_send_holds_open_only_the_response_being_read(cx: &mut gpui::TestAppContext) {
    let open = |view: &crate::view::AgentChatView, ix: usize| {
        view.rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::AgentItem(i) if i == ix) && !r.hidden)
    };
    let shown = |view: &crate::view::AgentChatView, ix: usize| {
        view.rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::ConclusionItem(i) if i == ix) && !r.hidden)
    };
    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            // 0: q1, 1: process, 2: final
            view.items.push(daruda_acp::ChatItem::UserText("q1".into()));
            view.items.push(assistant_text_item("process 1"));
            view.items.push(assistant_text_item("final 1"));
            view.rebuild_rows();
            assert!(open(view, 1), "the tail response starts expanded");

            // 3: q2 — the send must not hide the prose the user was reading.
            view.echo_prompt("q2".to_string(), cx);
            assert!(open(view, 1), "the send holds the response being read");
            assert!(shown(view, 2), "and its conclusion");

            // 4: process, 5: final, 6: q3 — the hold moves to the newest.
            view.items.push(assistant_text_item("process 2"));
            view.items.push(assistant_text_item("final 2"));
            view.echo_prompt("q3".to_string(), cx);
            assert!(
                !open(view, 1),
                "the older response folds once it is not being read"
            );
            assert!(open(view, 4), "the newly held response stays open");

            // 7: process, 8: final — collapsed by the user before 9: q4.
            view.items.push(assistant_text_item("process 3"));
            view.items.push(assistant_text_item("final 3"));
            view.toggle_fold(FoldKey::Response(7), window, cx);
            view.echo_prompt("q4".to_string(), cx);
            assert!(
                !open(view, 7),
                "a user collapse stays collapsed after the send"
            );
            assert!(shown(view, 8), "the conclusion still surfaces from it");

            // After three sends, a Mode still governs every response.
            view.items.push(assistant_text_item("process last"));
            view.items.push(assistant_text_item("final last"));
            view.rebuild_rows();
            view.set_fold_mode(FoldPreset::Summary.mode(), window, cx);
            let still_open: Vec<usize> = view
                .rows
                .iter()
                .filter(|r| !r.hidden)
                .filter_map(|r| match r.kind {
                    RowKind::AgentItem(ix) => Some(ix),
                    _ => None,
                })
                .collect();
            assert!(
                still_open.is_empty(),
                "Summary collapses every response's process prose; still open: {still_open:?}"
            );
        })
        .unwrap();
}

// The compact Activity Bar split and the repaints it costs.

#[gpui::test]
fn the_compact_split_is_measured_on_the_pane_not_the_list(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            let split = daruda_ui::theme::agent_chat_compact_options_w(cx);
            assert!(
                !view.activity_bar_is_compact(cx),
                "before the first paint measures anything, the three chips are assumed to fit"
            );

            view.set_pane_width(gpui::px(split - 1.0), cx);
            assert!(view.activity_bar_is_compact(cx));
            assert!(view.items.is_empty());
            assert!(view.list_bounds.is_none());

            view.items.push(assistant_text_item("now there is a list"));
            assert!(
                view.activity_bar_is_compact(cx),
                "gaining a transcript must not change which bar the pane shows"
            );

            view.set_pane_width(gpui::px(split + 1.0), cx);
            assert!(!view.activity_bar_is_compact(cx));
        })
        .expect("view update");
}

#[gpui::test]
fn only_a_measurement_that_flips_the_split_repaints(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    // The split is font-dependent; read it the way the pane does.
    let split = window
        .update(cx, |_view, _window, cx| {
            daruda_ui::theme::agent_chat_compact_options_w(cx)
        })
        .expect("view update");
    let wide = split + 1.0;
    let set = |cx: &mut gpui::TestAppContext, w: f32| {
        window
            .update(cx, |view, _window, cx| view.set_pane_width(gpui::px(w), cx))
            .expect("view update");
        cx.run_until_parked();
    };
    let count = |cx: &mut gpui::TestAppContext| {
        window
            .update(cx, |view, _window, _cx| view.render_count.get())
            .expect("view update")
    };

    set(cx, wide);
    let settled = count(cx);
    assert!(
        settled >= 1,
        "the pane painted at least once, got {settled}"
    );

    set(cx, wide + 40.0);
    set(cx, wide + 80.0);
    assert_eq!(
        count(cx),
        settled,
        "a resize that stays on the wide side is not a layout change"
    );

    set(cx, split - 1.0);
    assert!(
        count(cx) > settled,
        "crossing into the compact bar has to repaint, or the chips stay stale"
    );
}
