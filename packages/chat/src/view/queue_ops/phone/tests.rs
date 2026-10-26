use crate::view::tests::make_test_view;

#[gpui::test]
fn turn_end_resolves_telegram_watch_after_finalizing_streaming_text(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            view.start_phone_turn_for_test(std::time::Instant::now());
            view.items.push(daruda_acp::ChatItem::AssistantText {
                text: "done".to_string(),
                streaming: true,
                message_id: None,
                phase: Default::default(),
            });

            let effect = view.apply_event(
                daruda_acp::AcpEvent::TurnEnded {
                    completed_normally: true,
                    stop_reason: "EndTurn".into(),
                    usage: None,
                },
                "",
                false,
                cx,
            );

            assert_eq!(
                effect,
                crate::view::PhoneAckEffect::Relay(crate::view::FirstResponseOutcome::Text {
                    text: "done".to_string(),
                    message_id: None,
                })
            );
            assert!(
                !view.is_phone_turn_waiting(),
                "the resolved watch is cleared at the terminal boundary"
            );
        })
        .unwrap();
}
