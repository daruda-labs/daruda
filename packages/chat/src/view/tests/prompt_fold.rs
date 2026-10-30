//! Long prompts: open while their turn runs, folded from its settle edge.
use super::*;
use crate::fold::FoldKey;
use daruda_acp::{AcpEvent, ChatItem};

#[gpui::test]
fn a_long_prompt_folds_at_the_settle_and_the_next_prompt_stays_open(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            let long = "log line\n".repeat(40);
            view.items = vec![ChatItem::UserText(long), assistant_text_item("reading")];
            view.set_turn_in_flight();
            let expanded = |view: &super::super::AgentChatView, ix| {
                let key = FoldKey::UserPrompt(ix);
                view.fold.is_expanded(
                    &key,
                    crate::agent_chat_helpers::fold_context(&key, &view.items, &view.live_units),
                )
            };
            assert!(expanded(view, 0), "open while the agent works");

            view.apply_event(
                AcpEvent::TurnEnded {
                    stop_reason: "EndTurn".into(),
                    completed_normally: true,
                    usage: None,
                },
                "",
                false,
                cx,
            );
            assert!(!expanded(view, 0), "folded once its turn settled");

            // The next prompt lands after that settle, as a pumped or typed send
            // does; the settle already happened, so it must not fold with it.
            view.items.push(ChatItem::UserText("more log\n".repeat(40)));
            let next = view.items.len() - 1;
            assert!(
                expanded(view, next),
                "a prompt sent after the settle is open"
            );
        })
        .unwrap();
}

#[test]
fn a_preview_keeps_the_opening_lines_and_caps_one_huge_line() {
    use crate::render::prompt_preview_for_test as preview;
    let many = (1..=20)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let shown = preview(&many);
    assert!(shown.starts_with("line 1\nline 2"));
    assert!(!shown.contains("line 7"));
    assert!(shown.ends_with('…'));

    let one = "x".repeat(5_000);
    assert_eq!(
        preview(&one).chars().count(),
        crate::PROMPT_PREVIEW_CHARS + 1
    );
}

/// The user's path through a folded prompt: read the preview, open it, fold it
/// back — each click changing the row's height.
#[gpui::test]
fn the_toggle_opens_and_refolds_a_settled_prompt(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items = vec![
                ChatItem::UserText("pasted log line\n".repeat(40)),
                assistant_text_item("read it"),
            ];
            view.settle_prompts();
            view.rebuild_rows();
            cx.notify();
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let prompt_height = |visual: &mut gpui::VisualTestContext| {
        window
            .read_with(visual, |view, _| {
                let row = view
                    .rows
                    .iter()
                    .position(|r| matches!(r.kind, crate::rows::RowKind::User(0)))
                    .expect("prompt row");
                view.list_state
                    .bounds_for_item(row)
                    .expect("prompt row measured")
                    .size
                    .height
            })
            .unwrap()
    };
    let click_toggle = |visual: &mut gpui::VisualTestContext| {
        let toggle = visual
            .debug_bounds("agent-chat-user-fold-0")
            .expect("the toggle sits under the prompt");
        // Right-aligned in a full-width row: press just inside its right end.
        let at = gpui::point(toggle.right() - gpui::px(8.), toggle.center().y);
        visual.simulate_click(at, gpui::Modifiers::default());
        visual.run_until_parked();
    };

    let folded = prompt_height(&mut visual);
    click_toggle(&mut visual);
    let open = prompt_height(&mut visual);
    assert!(
        open > folded,
        "opening shows the whole prompt: {folded:?} -> {open:?}"
    );
    click_toggle(&mut visual);
    assert_eq!(
        prompt_height(&mut visual),
        folded,
        "folding it back restores the preview"
    );
}
