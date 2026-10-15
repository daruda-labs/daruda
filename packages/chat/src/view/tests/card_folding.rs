//! Regression coverage for nested card navigation through the rendered footer.
use super::*;
use crate::view::list_sync::CollapseAnchor;
use daruda_acp::{ChatItem, ToolKindView, ToolOutputBlock, ToolStatusView};
use gpui::{MouseButton, VisualTestContext, point, px};
use serde_json::json;

#[gpui::test]
fn nested_footer_closes_only_its_card_and_recovers_its_header(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            let mut parent = tool_call("parent", ToolStatusView::Completed, None);
            if let ChatItem::ToolCall(call) = &mut parent {
                call.kind = ToolKindView::Think;
                call.raw_input = Some(
                    serde_json::json!({"subagent_type":"reviewer", "prompt":"Review the changes."}),
                );
            }
            let mut child = tool_call("child", ToolStatusView::Completed, Some("parent"));
            if let ChatItem::ToolCall(call) = &mut child {
                call.output.push(ToolOutputBlock::Text {
                    text: "Retained paragraph.\n\n".repeat(40),
                    truncated_from: None,
                });
            }
            view.items = vec![
                parent,
                child,
                tool_call("sibling", ToolStatusView::Completed, Some("parent")),
                ChatItem::UserText("Later conversation.\n\n".repeat(40)),
            ];
            view.rebuild_rows();
            view.set_all_folds(true, window, cx);
            view.list_state.scroll_to(gpui::ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let footer = visual
        .debug_bounds("fold-footer-Tool(\"child\")")
        .expect("nested footer painted");
    window
        .update(&mut visual, |view, _, cx| {
            let row = crate::rows::item_row(&view.rows, 0).unwrap();
            let row_bounds = view.list_state.bounds_for_item(row).unwrap();
            let viewport = view.list_state.viewport_bounds();
            let offset = footer.center().y - row_bounds.top() - viewport.size.height / 2.;
            view.list_state.scroll_to(gpui::ListOffset {
                item_ix: row,
                offset_in_item: offset,
            });
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    let footer = visual
        .debug_bounds("fold-footer-Tool(\"child\")")
        .expect("nested footer visible");
    let click = point(footer.left() + px(12.), footer.center().y);
    visual.simulate_mouse_move(click, None, Default::default());
    visual.simulate_mouse_down(click, MouseButton::Left, Default::default());
    visual.simulate_mouse_up(click, MouseButton::Left, Default::default());
    visual.run_until_parked();
    window
        .update(&mut visual, |view, _, _| {
            for (key, expected) in [
                (FoldKey::Subagent("parent".into()), true),
                (FoldKey::Tool("child".into()), false),
                (FoldKey::Tool("sibling".into()), true),
            ] {
                let context =
                    crate::agent_chat_helpers::fold_context(&key, &view.items, &view.live_units);
                assert_eq!(view.fold.is_expanded(&key, context), expected, "{key:?}");
            }
            // The child starts below the parent's title and instructions.
            assert!(view.list_state.logical_scroll_top().offset_in_item > px(50.));
        })
        .unwrap();
}

#[gpui::test]
fn footer_keeps_an_already_visible_header_in_place(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            view.items = vec![
                tool_call("short", ToolStatusView::Completed, None),
                ChatItem::UserText("Following content.\n".repeat(40)),
            ];
            view.rebuild_rows();
            view.set_all_folds(true, window, cx);
            view.list_state.scroll_to(gpui::ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let before = window
        .update(&mut visual, |view, _, _| {
            view.list_state.logical_scroll_top()
        })
        .unwrap();
    let footer = visual
        .debug_bounds("fold-footer-Tool(\"short\")")
        .expect("footer painted");
    let click = point(footer.left() + px(12.), footer.center().y);
    visual.simulate_mouse_down(click, MouseButton::Left, Default::default());
    visual.simulate_mouse_up(click, MouseButton::Left, Default::default());
    visual.run_until_parked();
    window
        .update(&mut visual, |view, _, _| {
            let after = view.list_state.logical_scroll_top();
            assert_eq!(after.item_ix, before.item_ix);
            assert_eq!(after.offset_in_item, before.offset_in_item);
            assert!(
                !view
                    .fold
                    .is_expanded(&FoldKey::Tool("short".into()), FoldContext::last(false))
            );
        })
        .unwrap();
}

#[gpui::test]
fn card_footer_collapses_tools_and_subagents_without_toggling_them_open(
    cx: &mut gpui::TestAppContext,
) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            let tool = tool_call("tool", daruda_acp::ToolStatusView::Completed, None);
            let mut agent = tool_call("agent", daruda_acp::ToolStatusView::Completed, None);
            if let ChatItem::ToolCall(call) = &mut agent {
                call.kind = daruda_acp::ToolKindView::Think;
                call.raw_input = Some(json!({"subagent_type":"reviewer"}));
            }
            view.items = vec![tool, agent];
            view.rebuild_rows();
            for key in [
                FoldKey::Tool("tool".into()),
                FoldKey::Subagent("agent".into()),
            ] {
                view.fold.set_all([key.clone()], true);
                view.collapse_card(key.clone(), CollapseAnchor::Unmeasured, window, cx);
                let context =
                    crate::agent_chat_helpers::fold_context(&key, &view.items, &view.live_units);
                assert!(!view.fold.is_expanded(&key, context));
                view.collapse_card(key.clone(), CollapseAnchor::Unmeasured, window, cx);
                assert!(!view.fold.is_expanded(&key, context));
            }
        })
        .unwrap();
}
