use crate::view::tests::make_test_view;

#[gpui::test]
fn protocol_notices_do_not_change_transcript_turn_or_cancel_ack(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{AcpEvent, NoticeSeverityView, SessionNoticeView};
    let window = make_test_view(cx);
    window
        .update(cx, |view, _, cx| {
            view.set_turn_in_flight();
            for severity in [
                NoticeSeverityView::Info,
                NoticeSeverityView::Error,
                NoticeSeverityView::Other("future".into()),
            ] {
                let items = view.items.clone();
                let outcome = view.activity.pending_completion.clone();
                view.apply_event(
                    AcpEvent::SessionNotice {
                        session_id: "root".into(),
                        notice: SessionNoticeView {
                            severity,
                            title: "Advisory".into(),
                            description: None,
                        },
                    },
                    "",
                    false,
                    cx,
                );
                assert!(!view.turn_is_idle());
                assert_eq!(view.items, items);
                assert_eq!(view.activity.pending_completion, outcome);
            }
            view.cancel_turn(cx);
            assert!(view.turn_awaiting_cancel_ack());
            let items = view.items.clone();
            view.apply_event(
                AcpEvent::SessionNotice {
                    session_id: "child".into(),
                    notice: SessionNoticeView {
                        severity: NoticeSeverityView::Error,
                        title: "Still advisory".into(),
                        description: None,
                    },
                },
                "",
                false,
                cx,
            );
            assert!(view.turn_awaiting_cancel_ack());
            assert_eq!(view.items, items);
            assert_eq!(
                view.activity.pending_completion,
                Some(crate::view::TurnOutcome::Stopped)
            );
        })
        .unwrap();
}
