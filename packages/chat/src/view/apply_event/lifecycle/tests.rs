use crate::view::tests::make_test_view;

/// A `session/prompt` JSON-RPC failure keeps the ACP session alive: it shows an
/// inline error, settles the foreground turn, and records an errored completion
/// without switching the pane to terminal `Error`.
#[gpui::test]
fn turn_failed_keeps_session_connected_and_shows_error(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{AcpEvent, ChatItem};

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.status = crate::view::AgentSessionStatus::Connected;
            view.set_turn_in_flight();

            view.apply_event(
                AcpEvent::TurnFailed(daruda_acp::AcpFailure::unclassified(
                    "session limit reached",
                )),
                "",
                false,
                cx,
            );

            assert!(
                matches!(view.status, crate::view::AgentSessionStatus::Connected),
                "a per-turn failure must not kill the session, got {:?}",
                view.status
            );
            assert!(view.turn_is_idle(), "the failed turn must settle to idle");
            assert_eq!(
                view.items.last(),
                Some(&ChatItem::Failure(daruda_acp::AcpFailure::unclassified(
                    "session limit reached"
                ))),
                "the failure is surfaced as an inline Error item"
            );
            assert_eq!(
                view.activity.pending_completion,
                Some(crate::view::TurnOutcome::Errored)
            );
        })
        .unwrap();
}
