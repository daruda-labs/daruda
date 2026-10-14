//! The overage stop: once per turn, only on a busy pane, and its banner gone
//! with the next prompt that reaches the wire.

use super::*;
use daruda_acp::{RateLimitView, RateLimitWindow};

fn billed_limit() -> RateLimitView {
    RateLimitView {
        window: Some(RateLimitWindow::FiveHour),
        resets_at: Some(1_800_000_000),
        is_using_overage: true,
    }
}

#[gpui::test]
fn overage_stop_cancels_a_busy_turn_once_and_keeps_the_notice(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            assert!(view.stop_for_overage(billed_limit(), cx));

            assert!(view.turn_is_idle(), "the stop settles the turn locally");
            assert_eq!(
                view.activity.pending_completion,
                Some(crate::view::TurnOutcome::Stopped),
                "it is the same Stop the button issues"
            );
            let notice = view.overage_notice().expect("notice set").clone();
            assert_eq!(notice.window, Some(RateLimitWindow::FiveHour));
            assert_eq!(notice.resets_at, Some(1_800_000_000));

            // A later billed update in the same turn must not cancel again.
            let markers = |v: &crate::view::AgentChatView| {
                v.items
                    .iter()
                    .filter(|i| matches!(i, daruda_acp::ChatItem::Interrupted))
                    .count()
            };
            assert_eq!(markers(view), 1);
            view.set_turn_in_flight();
            assert!(!view.stop_for_overage(billed_limit(), cx));
            assert!(!view.turn_is_idle(), "a second call is a no-op");
            assert_eq!(markers(view), 1, "no second stop marker");
        })
        .unwrap();
}

#[gpui::test]
fn overage_stop_on_an_idle_pane_does_nothing(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            assert!(!view.stop_for_overage(billed_limit(), cx));
            assert!(view.overage_notice().is_none());
            assert!(view.items.is_empty(), "no stop marker on an idle pane");
            assert_eq!(view.activity.pending_completion, None);
        })
        .unwrap();
}

#[gpui::test]
fn the_next_dispatched_prompt_clears_the_notice(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            assert!(view.stop_for_overage(billed_limit(), cx));
            view.queue.turn.acknowledge_cancel();
            view.queue.pending_prompts.push(queued(1, "next"));

            assert!(view.overage_notice().is_some());
            assert_eq!(
                view.drain_next_queued_prompt_for_test(cx).as_deref(),
                Some("next")
            );
            assert!(view.overage_notice().is_none(), "a new turn retires it");
        })
        .unwrap();
}

fn marker_ixs(view: &crate::view::AgentChatView) -> Vec<usize> {
    view.items
        .iter()
        .enumerate()
        .filter(|(_, i)| matches!(i, daruda_acp::ChatItem::Interrupted))
        .map(|(ix, _)| ix)
        .collect()
}

#[gpui::test]
fn an_overage_stop_marker_says_why_the_run_stopped(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items = vec![daruda_acp::ChatItem::UserText("go".into())];
            view.set_turn_in_flight();
            assert!(view.stop_for_overage(billed_limit(), cx));

            let ix = *marker_ixs(view).last().expect("a marker");
            assert!(view.overage_stop_markers.contains(&ix));
            assert_eq!(view.stop_marker_label(ix), "Stopped to avoid extra usage");
        })
        .unwrap();
}

#[gpui::test]
fn a_manual_stop_keeps_the_user_label_and_leaves_earlier_ones(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items = vec![daruda_acp::ChatItem::UserText("go".into())];
            view.set_turn_in_flight();
            assert!(view.stop_for_overage(billed_limit(), cx));
            view.queue.turn.acknowledge_cancel();

            view.items
                .push(daruda_acp::ChatItem::UserText("again".into()));
            view.set_turn_in_flight();
            view.cancel_turn(cx);

            let [overage, manual] = marker_ixs(view)[..] else {
                panic!("two markers expected");
            };
            assert_eq!(view.stop_marker_label(manual), "Stopped by user");
            assert_eq!(
                view.stop_marker_label(overage),
                "Stopped to avoid extra usage",
                "a later Stop does not relabel the earlier overage marker"
            );
        })
        .unwrap();
}

#[gpui::test]
fn the_settled_overage_marker_keeps_its_label(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items = vec![daruda_acp::ChatItem::UserText("go".into())];
            view.set_turn_in_flight();
            assert!(view.stop_for_overage(billed_limit(), cx));
            // A chunk that beat the cancel ack; the ack moves the marker below it.
            view.items.push(assistant_text_item("half a sentence"));
            view.apply_event(
                daruda_acp::AcpEvent::TurnEnded {
                    completed_normally: false,
                    stop_reason: "Cancelled".into(),
                    usage: None,
                },
                "",
                false,
                cx,
            );

            let last = view.items.len() - 1;
            assert_eq!(marker_ixs(view), vec![last]);
            assert_eq!(
                view.overage_stop_markers
                    .iter()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![last]
            );
            assert_eq!(view.stop_marker_label(last), "Stopped to avoid extra usage");
        })
        .unwrap();
}

/// Turn already idle, trailing subagents still running: like the Stop button
/// in that state, the overage stop cuts nothing the replay would record.
#[gpui::test]
fn an_overage_stop_behind_trailing_subagents_marks_nothing(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_idle();
            view.items = vec![tool_call(
                "child",
                daruda_acp::ToolStatusView::InProgress,
                Some("parent"),
            )];
            view.activity.pending_completion = Some(crate::view::TurnOutcome::Completed);
            view.queue.pending_prompts.push(queued(1, "next"));
            assert!(view.is_busy(), "the running child keeps the pane busy");

            assert!(view.stop_for_overage(billed_limit(), cx));

            assert!(marker_ixs(view).is_empty(), "no Interrupted marker");
            assert_eq!(
                view.activity.pending_completion,
                Some(crate::view::TurnOutcome::Completed),
                "no Stopped outcome replaces the captured one"
            );
            assert!(view.queue.pending_prompts.is_empty());
            assert_eq!(texts(&view.queue.paused_prompts), vec!["next".to_string()]);
            assert!(view.overage_notice().is_some(), "the banner is shown");
        })
        .unwrap();
}

/// A fresh session keeps the transcript, so it keeps what its markers say.
#[gpui::test]
fn a_fresh_session_keeps_the_overage_marker_label(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items = vec![daruda_acp::ChatItem::UserText("go".into())];
            view.set_turn_in_flight();
            assert!(view.stop_for_overage(billed_limit(), cx));
            view.apply_event(
                daruda_acp::AcpEvent::Connected {
                    program: None,
                    session_id: "fresh".into(),
                    modes: None,
                    config_options: Vec::new(),
                    capabilities: Default::default(),
                    login_methods: Vec::new(),
                },
                "",
                false,
                cx,
            );

            assert!(view.overage_notice().is_none(), "the banner goes");
            let ix = *marker_ixs(view).last().expect("the marker survives");
            assert_eq!(view.stop_marker_label(ix), "Stopped to avoid extra usage");
        })
        .unwrap();
}

/// A user Stop whose trailing subagent keeps the pane busy stays the user's:
/// a late billed update neither cancels again nor raises the banner.
#[gpui::test]
fn a_billed_update_after_a_user_stop_leaves_it_alone(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items = vec![daruda_acp::ChatItem::UserText("go".into())];
            view.set_turn_in_flight();
            view.cancel_turn(cx);
            view.items.push(tool_call(
                "child",
                daruda_acp::ToolStatusView::InProgress,
                Some("parent"),
            ));
            assert!(view.is_busy(), "the running child keeps the pane busy");

            assert!(!view.stop_for_overage(billed_limit(), cx), "awaiting ack");
            view.queue.turn.acknowledge_cancel();
            assert!(!view.stop_for_overage(billed_limit(), cx), "ack landed");

            assert!(view.overage_notice().is_none());
            let [ix] = marker_ixs(view)[..] else {
                panic!("one marker expected");
            };
            assert_eq!(view.stop_marker_label(ix), "Stopped by user");
        })
        .unwrap();
}

/// A prompt dispatched after that Stop's ack, while the child still runs, is
/// a new turn: its billed update stops it despite the stale `Stopped`.
#[gpui::test]
fn a_new_turn_after_a_user_stop_with_a_trailing_child_still_stops(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items = vec![daruda_acp::ChatItem::UserText("go".into())];
            view.set_turn_in_flight();
            view.cancel_turn(cx);
            view.items.push(tool_call(
                "child",
                daruda_acp::ToolStatusView::InProgress,
                Some("parent"),
            ));
            view.queue.turn.acknowledge_cancel();
            view.queue.pending_prompts.push(queued(1, "next"));
            assert_eq!(
                view.drain_next_queued_prompt_for_test(cx).as_deref(),
                Some("next")
            );
            assert_eq!(
                view.activity.pending_completion,
                Some(crate::view::TurnOutcome::Stopped),
                "the busy pane never crossed the idle edge"
            );
            assert!(!view.turn_is_idle(), "the new turn is in flight");

            assert!(view.stop_for_overage(billed_limit(), cx));
            assert!(view.turn_is_idle(), "the new turn is cancelled");
            assert!(view.overage_notice().is_some());
            let ix = *marker_ixs(view).last().expect("a marker");
            assert_eq!(view.stop_marker_label(ix), "Stopped to avoid extra usage");
        })
        .unwrap();
}
