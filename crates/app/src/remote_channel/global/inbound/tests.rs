use super::*;
use crate::remote_channel::command::RenderedReply;

fn callback() -> IncomingKind {
    IncomingKind::Callback {
        data: "token".into(),
        message_id: "12.34".into(),
        original: "Allow write?".into(),
    }
}

/// The defect this guards: a tap whose token is unknown or already consumed
/// used to get a fresh message, leaving the dead buttons on screen for the
/// user to keep tapping.
#[test]
fn a_dead_tap_is_answered_in_place_and_loses_its_buttons() {
    let Feedback::Edit {
        message_id,
        original,
        label,
    } = feedback_for(Effect::None, callback())
    else {
        panic!("a tapped button must be answered in place");
    };
    assert_eq!(message_id, "12.34");
    assert_eq!(original, "Allow write?");
    assert!(!label.is_empty());
}

#[test]
fn a_settled_decision_consumes_its_buttons_and_a_selection_keeps_them() {
    assert!(matches!(
        feedback_for(
            Effect::Feedback {
                label: "Allowed".into(),
                edit: Edit::ConsumeButtons,
            },
            callback(),
        ),
        Feedback::Edit { .. }
    ));
    // A listing row stays tappable — picking another row is ordinary use.
    assert_eq!(
        feedback_for(
            Effect::Feedback {
                label: "Now talking to lane 2".into(),
                edit: Edit::KeepButtons,
            },
            callback(),
        ),
        Feedback::Send(Message::plain("Now talking to lane 2".into(), None)),
    );
}

/// A plain message has no keyboard to strip, so nothing is owed unless the
/// effect produced an answer of its own.
#[test]
fn a_plain_message_is_never_edited() {
    let text = IncomingKind::Message {
        text: "ship it".into(),
        reply_to: None,
    };
    assert_eq!(feedback_for(Effect::None, text), Feedback::Nothing);
    assert_eq!(
        feedback_for(
            Effect::Reply(RenderedReply {
                text: "done".into(),
                keyboard: None,
            }),
            IncomingKind::Message {
                text: "ship it".into(),
                reply_to: None,
            },
        ),
        Feedback::Send(Message::plain("done".into(), None)),
    );
}
