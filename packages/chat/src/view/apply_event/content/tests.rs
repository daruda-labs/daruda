use crate::fold::{FoldContext, FoldKey};
use crate::view::list_sync::CollapseAnchor;
use crate::view::tests::make_test_view;
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
fn compaction_expands_without_a_height_cap_and_footer_recovers_header(
    cx: &mut gpui::TestAppContext,
) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            let mut value = update("completed");
            value["summary"] =
                json!([{"type":"text", "text":"Retained paragraph.\n\n".repeat(40)}]);
            view.apply_event(event("root", value), "", false, cx);
            view.toggle_fold(crate::fold::FoldKey::Compaction(0), window, cx);
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let body = visual
        .debug_bounds("agent-chat-compaction-summary-0")
        .expect("summary painted");
    assert!(body.size.height > gpui::px(800.));
    window
        .update(&mut visual, |view, window, cx| {
            let key = crate::fold::FoldKey::Compaction(0);
            let row = crate::rows::item_row(&view.rows, 0).unwrap();
            view.list_state.scroll_to(gpui::ListOffset {
                item_ix: row,
                offset_in_item: gpui::px(600.),
            });
            view.collapse_card(key.clone(), CollapseAnchor::Unmeasured, window, cx);
            assert!(
                !view
                    .fold
                    .is_expanded(&key, crate::fold::FoldContext::last(false))
            );
            assert_eq!(
                view.list_state.logical_scroll_top().offset_in_item,
                gpui::px(0.)
            );
            view.collapse_card(key.clone(), CollapseAnchor::Unmeasured, window, cx);
            assert!(
                !view
                    .fold
                    .is_expanded(&key, crate::fold::FoldContext::last(false))
            );
        })
        .unwrap();
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

#[gpui::test]
fn compaction_footer_click_recovers_the_rendered_header(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            let mut value = update("completed");
            value["summary"] =
                json!([{"type":"text", "text":"Retained paragraph.\n\n".repeat(40)}]);
            view.apply_event(event("root", value), "", false, cx);
            view.toggle_fold(FoldKey::Compaction(0), window, cx);
            view.list_state.scroll_to_end();
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let footer = visual
        .debug_bounds("fold-footer-Compaction(0)")
        .expect("footer painted");
    let click = gpui::point(footer.left() + gpui::px(12.), footer.center().y);
    visual.simulate_mouse_move(click, None, Default::default());
    visual.simulate_mouse_down(click, gpui::MouseButton::Left, Default::default());
    visual.simulate_mouse_up(click, gpui::MouseButton::Left, Default::default());
    visual.run_until_parked();
    window
        .update(&mut visual, |view, _, _| {
            assert!(
                !view
                    .fold
                    .is_expanded(&FoldKey::Compaction(0), FoldContext::last(false))
            );
            let row = crate::rows::item_row(&view.rows, 0).unwrap();
            let bounds = view
                .list_state
                .bounds_for_item(row)
                .expect("collapsed card visible");
            assert!(bounds.bottom() > view.list_state.viewport_bounds().top());
            assert!(view.list_state.logical_scroll_top().offset_in_item < gpui::px(50.));
        })
        .unwrap();
}
