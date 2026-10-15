use super::*;
use daruda_acp::{AcpEvent, ChatItem, CompactionStatusView};
use serde_json::json;

fn event(session: &str, value: serde_json::Value) -> AcpEvent {
    AcpEvent::Compaction {
        session_id: session.into(),
        update: Box::new(serde_json::from_value(value).unwrap()),
    }
}

fn update(status: &str) -> serde_json::Value {
    json!({"sessionUpdate":"compaction_update", "compactionId":"same", "status":status})
}

#[gpui::test]
fn compaction_preserves_turn_cancel_ack_usage_and_first_seen_row(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _, cx| {
            view.items.push(ChatItem::UserText("prompt".into()));
            view.set_turn_in_flight();
            view.apply_event(
                AcpEvent::UsageChanged(daruda_acp::UsageView {
                    used: 50,
                    size: 100,
                    cost: None,
                    rate_limit: None,
                }),
                "",
                false,
                cx,
            );
            view.apply_event(event("root", update("in_progress")), "", false, cx);
            let first_row = crate::rows::item_row(&view.rows, 1).unwrap();
            view.list_state.scroll_to(gpui::ListOffset {
                item_ix: first_row,
                offset_in_item: gpui::px(7.),
            });
            view.items.push(ChatItem::UserText("later".into()));
            for status in ["completed", "failed", "cancelled", "future"] {
                view.apply_event(event("root", update(status)), "", false, cx);
                assert!(!view.turn_is_idle());
                assert_eq!(view.activity.pending_completion, None);
                assert_eq!(view.session_usage.as_ref().unwrap().used, 50);
                assert_eq!(crate::rows::item_row(&view.rows, 1), Some(first_row));
                let anchor = view.list_state.logical_scroll_top();
                assert_eq!(anchor.item_ix, first_row);
                assert_eq!(anchor.offset_in_item, gpui::px(7.));
            }
            assert_eq!(view.items.len(), 3);
            view.cancel_turn(cx);
            view.apply_event(event("child", update("in_progress")), "", false, cx);
            view.apply_event(event("child", update("failed")), "", false, cx);
            assert!(view.turn_awaiting_cancel_ack());
            assert_eq!(
                view.activity.pending_completion,
                Some(crate::view::TurnOutcome::Stopped)
            );
            let ChatItem::Compaction(child) = view.items.last().unwrap() else {
                panic!("child compaction")
            };
            assert_eq!(child.session_id, "child");
            assert_eq!(child.status, CompactionStatusView::Failed);
        })
        .unwrap();
}

#[gpui::test]
fn compaction_stays_visible_when_prose_is_filtered_or_responses_are_collapsed(
    cx: &mut gpui::TestAppContext,
) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            view.items.push(ChatItem::UserText("prompt".into()));
            let mut value = update("completed");
            value["summary"] = json!([{"type":"text", "text":"Retained context."}]);
            view.apply_event(event("root", value), "", false, cx);
            view.items.push(ChatItem::AssistantText {
                text: "after compaction".into(),
                streaming: false,
                message_id: None,
                phase: daruda_acp::adapter::MessagePhase::Answer,
            });
            view.display_filter = crate::pane_choice::PaneChoice::Chosen(
                daruda_transcript::display_filter::DisplayFilter::from_tokens([]),
            );
            view.fold
                .set_mode(daruda_transcript::fold_mode::FoldPreset::Summary.mode());
            view.rebuild_rows();
            let row = crate::rows::item_row(&view.rows, 1).unwrap();
            assert!(!view.rows[row].hidden);
            assert_eq!(view.rows[row].indent, 0);
            assert!(
                crate::rows::collect_foldable_keys(&view.items)
                    .contains(&crate::fold::FoldKey::Compaction(1))
            );
            let key = crate::fold::FoldKey::Compaction(1);
            let context =
                crate::agent_chat_helpers::fold_context(&key, &view.items, &view.live_units);
            assert!(!view.fold.is_expanded(&key, context));
            view.toggle_fold(key.clone(), window, cx);
            assert!(view.fold.is_expanded(&key, context));
            view.apply_event(event("root", update("failed")), "", false, cx);
            assert!(view.fold.is_expanded(&key, context));
        })
        .unwrap();
}

#[gpui::test]
fn compaction_does_not_create_a_second_response_or_conclusion(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _, cx| {
            let prose = |text: &str| ChatItem::AssistantText {
                text: text.into(),
                streaming: false,
                message_id: None,
                phase: daruda_acp::adapter::MessagePhase::Answer,
            };
            view.items = vec![ChatItem::UserText("prompt".into()), prose("before")];
            view.apply_event(event("root", update("completed")), "", false, cx);
            view.items.push(prose("after"));
            view.rebuild_rows();
            assert_eq!(
                view.rows
                    .iter()
                    .filter(|row| matches!(row.kind, crate::rows::RowKind::ResponseHeader { .. }))
                    .count(),
                1
            );
            let conclusions: Vec<_> = view
                .rows
                .iter()
                .filter_map(|row| match row.kind {
                    crate::rows::RowKind::ConclusionItem(ix) => Some(ix),
                    _ => None,
                })
                .collect();
            assert_eq!(conclusions, vec![3]);
            let row = crate::rows::item_row(&view.rows, 2).unwrap();
            assert!(!view.rows[row].hidden);
            assert_eq!(view.rows[row].indent, 0);
        })
        .unwrap();
}

#[gpui::test]
fn expanded_summary_has_a_bounded_scrollbar_over_the_shipped_body(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    let cap = window
        .update(cx, |view, window, cx| {
            let mut value = update("completed");
            value["summary"] =
                json!([{"type":"text", "text":"Retained paragraph.\n\n".repeat(200)}]);
            view.apply_event(event("root", value), "", false, cx);
            view.toggle_fold(crate::fold::FoldKey::Compaction(0), window, cx);
            view.toggle_compaction_summary(0, cx);
            daruda_ui::theme::agent_chat_font_size(cx)
                * daruda_ui::theme::MD_VIEW_LINE_HEIGHT
                * daruda_ui::theme::AGENT_CHAT_EMBED_MAX_ROWS as f32
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let wrapper = visual
        .debug_bounds("agent-chat-compaction-summary-0-wrapper")
        .expect("summary painted");
    let bar = visual
        .debug_bounds("agent-chat-compaction-summary-0-scrollbar-layer")
        .expect("scrollbar painted");
    assert!(wrapper.size.height > gpui::px(0.));
    assert!(wrapper.size.height <= gpui::px(cap));
    assert_eq!(bar, wrapper, "scrollbar stays inside the summary region");
}

#[gpui::test]
fn preview_is_bounded_without_an_inner_scrollbar(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    let cap = window
        .update(cx, |view, window, cx| {
            let mut value = update("completed");
            value["summary"] =
                json!([{"type":"text", "text":"Retained paragraph.\n\n".repeat(200)}]);
            view.apply_event(event("root", value), "", false, cx);
            view.toggle_fold(crate::fold::FoldKey::Compaction(0), window, cx);
            daruda_ui::theme::agent_chat_font_size(cx)
                * daruda_ui::theme::MD_VIEW_LINE_HEIGHT
                * daruda_ui::theme::AGENT_CHAT_COMPACTION_PREVIEW_ROWS as f32
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let preview = visual
        .debug_bounds("agent-chat-compaction-preview-0")
        .expect("preview painted");
    assert!(preview.size.height > gpui::px(0.));
    // GPUI snaps the fractional line budget to physical pixel edges.
    assert!(preview.size.height <= gpui::px(cap.ceil()));
    assert!(
        visual
            .debug_bounds("agent-chat-compaction-summary-0-scrollbar-layer")
            .is_none()
    );
}

#[gpui::test]
fn failure_without_summary_is_passive_and_shows_its_reason(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _, cx| {
            let mut value = update("failed");
            value["error"] = json!("The provider could not produce a summary.");
            view.apply_event(event("root", value), "", false, cx);
            assert!(
                !crate::rows::collect_foldable_keys(&view.items)
                    .contains(&crate::fold::FoldKey::Compaction(0))
            );
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds("agent-chat-compaction-passive-0")
            .is_some()
    );
    assert!(
        visual
            .debug_bounds("agent-chat-compaction-error-0")
            .is_some()
    );
}
