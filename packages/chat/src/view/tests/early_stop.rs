//! A turn the agent cut short: what the transcript and the outcome say.
use super::*;
use daruda_acp::{AcpEvent, AcpFailure, ChatItem, EarlyStop};

fn end_turn(
    view: &mut super::super::AgentChatView,
    reason: &str,
    cx: &mut gpui::Context<super::super::AgentChatView>,
) {
    view.apply_event(
        AcpEvent::TurnEnded {
            stop_reason: reason.into(),
            completed_normally: true,
            usage: None,
        },
        "",
        false,
        cx,
    );
}

#[gpui::test]
fn an_early_stop_leaves_its_reason_and_settles_as_an_error(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            for (reason, stop) in [
                ("MaxTokens", EarlyStop::MaxTokens),
                ("MaxTurnRequests", EarlyStop::MaxTurnRequests),
                ("Refusal", EarlyStop::Refusal),
            ] {
                view.items = vec![
                    ChatItem::UserText("go".into()),
                    assistant_text_item("half a sente"),
                ];
                view.set_turn_in_flight();
                end_turn(view, reason, cx);
                assert_eq!(
                    view.items.last(),
                    Some(&ChatItem::Failure(AcpFailure::Stopped(stop))),
                    "{reason} is said in the transcript"
                );
                assert_eq!(
                    view.activity.pending_completion,
                    Some(super::super::TurnOutcome::Errored),
                    "{reason} does not settle a task as done"
                );
            }
        })
        .unwrap();
}

#[gpui::test]
fn a_finished_answer_adds_nothing(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items = vec![ChatItem::UserText("go".into()), assistant_text_item("done")];
            view.set_turn_in_flight();
            end_turn(view, "EndTurn", cx);
            assert!(
                !view.items.iter().any(|i| matches!(i, ChatItem::Failure(_))),
                "an answer the agent finished carries no notice"
            );
            assert_eq!(
                view.activity.pending_completion,
                Some(super::super::TurnOutcome::Completed)
            );
        })
        .unwrap();
}

#[gpui::test]
fn continue_is_a_visible_prompt_and_never_jumps_the_queue(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items = vec![
                ChatItem::UserText("go".into()),
                ChatItem::Failure(AcpFailure::Stopped(EarlyStop::MaxTokens)),
            ];
            view.enqueue_prompt("mine first".into(), crate::view::PromptOrigin::InApp)
                .expect("room in the queue");
            assert!(view.offers_continue());
            view.continue_after_stop(cx);
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec![
                    "mine first".to_string(),
                    crate::strings::agent_chat::continue_prompt()
                ],
                "it waits behind the user's own prompt"
            );

            view.set_turn_in_flight();
            assert!(!view.offers_continue(), "not while anything still works");
        })
        .unwrap();
}

/// The stop row loses its Continue button once something follows it. On screen
/// the list relays it out anyway; the splice in `rebuild_rows` also covers the
/// off-screen case, where a stale height would skew the scroll extent.
#[gpui::test]
fn the_stop_row_shrinks_when_its_button_goes(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items = vec![
                ChatItem::UserText("go".into()),
                ChatItem::Failure(AcpFailure::Stopped(EarlyStop::MaxTokens)),
            ];
            view.rebuild_rows();
            cx.notify();
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let height = |visual: &mut gpui::VisualTestContext| {
        window
            .read_with(visual, |view, _| {
                let row = crate::rows::item_row(&view.rows, 1).expect("stop row");
                view.list_state
                    .bounds_for_item(row)
                    .expect("stop row measured")
                    .size
                    .height
            })
            .unwrap()
    };
    let with_button = height(&mut visual);

    window
        .update(&mut visual, |view, _window, cx| {
            view.items.push(ChatItem::UserText("next".into()));
            view.rebuild_rows();
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(
        height(&mut visual) < with_button,
        "the row is remeasured without the button: {with_button:?}"
    );
}
