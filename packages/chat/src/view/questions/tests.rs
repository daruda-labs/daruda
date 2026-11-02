use daruda_acp::{
    AcpEvent, AcpSessionHandle, QuestionAnswer, QuestionField, QuestionFieldKind, QuestionOption,
    QuestionRequest, QuestionValue,
};

use crate::view::ActivityState;
use crate::view::tests::make_test_view;

fn option(value: &str) -> QuestionOption {
    QuestionOption {
        value: value.into(),
        label: value.into(),
        description: None,
    }
}

fn field(key: &str, kind: QuestionFieldKind, companion_of: Option<&str>) -> QuestionField {
    QuestionField {
        key: key.into(),
        title: None,
        description: None,
        required: false,
        kind,
        companion_of: companion_of.map(str::to_owned),
    }
}

/// `AskUserQuestion`'s one-question shape: a select and its "Other" box.
fn color_question() -> QuestionRequest {
    QuestionRequest {
        message: "Which color?".into(),
        tool_call_id: Some("toolu_1".into()),
        fields: vec![
            field(
                "question_0",
                QuestionFieldKind::SingleSelect(vec![option("red"), option("blue")]),
                None,
            ),
            field(
                "question_0_custom",
                QuestionFieldKind::Text,
                Some("question_0"),
            ),
        ],
    }
}

/// A view with a live session, and the receiver each parked id answers on.
fn asked(
    cx: &mut gpui::TestAppContext,
    questions: Vec<(u64, QuestionRequest)>,
) -> (
    gpui::WindowHandle<crate::view::AgentChatView>,
    Vec<daruda_acp::QuestionProbe>,
) {
    let window = make_test_view(cx);
    let (handle, _probe) = AcpSessionHandle::detached_for_test();
    let answers = questions
        .iter()
        .map(|(id, _)| handle.park_question_for_test(*id))
        .collect();
    window
        .update(cx, |view, _window, cx| {
            view.status = crate::view::AgentSessionStatus::Connected;
            view.attach_handle(handle);
            for (id, question) in questions {
                view.apply_event(AcpEvent::QuestionAsked { id, question }, "", false, cx);
            }
        })
        .unwrap();
    (window, answers)
}

fn answered(probe: &mut daruda_acp::QuestionProbe) -> Option<QuestionAnswer> {
    probe.answer()
}

/// A pending question is the pane's activity until it is answered, and a
/// single-select form sends on the pick.
#[gpui::test]
fn picking_the_only_select_sends_the_answer(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(cx, vec![(1, color_question())]);
    window
        .update(cx, |view, _window, cx| {
            assert_eq!(view.activity_state(), ActivityState::AwaitingAnswer);
            view.pick_question_option(1, "question_0".into(), "blue".into(), cx);
            assert!(!view.has_pending_question());
            assert_ne!(view.activity_state(), ActivityState::AwaitingAnswer);
        })
        .unwrap();
    let mut expected = std::collections::BTreeMap::new();
    expected.insert("question_0".into(), QuestionValue::Text("blue".into()));
    assert_eq!(
        answered(&mut answers[0]),
        Some(QuestionAnswer::Submit(expected))
    );
}

/// A typed answer is the select's answer; Send waits for it, then sends it.
#[gpui::test]
fn a_typed_answer_waits_for_send(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(cx, vec![(1, color_question())]);
    window
        .update(cx, |view, live_window, cx| {
            view.open_question_text(1, "question_0_custom".into(), live_window, cx);
            view.submit_question(1, cx);
            assert!(view.has_pending_question(), "nothing typed, nothing sent");
            let input = view
                .questions()
                .first()
                .and_then(|q| q.text("question_0_custom"))
                .cloned()
                .expect("the box is open");
            input.update(cx, |input, cx| input.set_value("green", live_window, cx));
            view.submit_question(1, cx);
            assert!(!view.has_pending_question());
        })
        .unwrap();
    let mut expected = std::collections::BTreeMap::new();
    expected.insert(
        "question_0_custom".into(),
        QuestionValue::Text("green".into()),
    );
    assert_eq!(
        answered(&mut answers[0]),
        Some(QuestionAnswer::Submit(expected))
    );
}

/// Several fields wait for Send; a required one blocks it until answered.
#[gpui::test]
fn a_multi_field_form_sends_on_send(cx: &mut gpui::TestAppContext) {
    let mut required = field(
        "features",
        QuestionFieldKind::MultiSelect(vec![option("a"), option("b")]),
        None,
    );
    required.required = true;
    let question = QuestionRequest {
        message: "Pick".into(),
        tool_call_id: None,
        fields: vec![required, field("confirm", QuestionFieldKind::Boolean, None)],
    };
    let (window, mut answers) = asked(cx, vec![(1, question)]);
    window
        .update(cx, |view, _window, cx| {
            view.set_question_flag(1, "confirm".into(), true, cx);
            view.submit_question(1, cx);
            assert!(view.has_pending_question(), "the required field is empty");
            view.pick_question_option(1, "features".into(), "a".into(), cx);
            view.pick_question_option(1, "features".into(), "b".into(), cx);
            view.pick_question_option(1, "features".into(), "a".into(), cx);
            assert!(
                view.has_pending_question(),
                "a pick does not send this form"
            );
            view.submit_question(1, cx);
        })
        .unwrap();
    let mut expected = std::collections::BTreeMap::new();
    expected.insert("features".into(), QuestionValue::Choices(vec!["b".into()]));
    expected.insert("confirm".into(), QuestionValue::Bool(true));
    assert_eq!(
        answered(&mut answers[0]),
        Some(QuestionAnswer::Submit(expected))
    );
}

/// Skip declines; Stop cancels whatever is still open, oldest first shown.
#[gpui::test]
fn skip_declines_and_stop_cancels_the_rest(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(cx, vec![(1, color_question()), (2, color_question())]);
    window
        .update(cx, |view, _window, cx| {
            assert_eq!(view.questions().len(), 2);
            assert_eq!(view.questions().first().map(|q| q.id), Some(1));
            view.skip_question(1, cx);
            assert_eq!(view.questions().first().map(|q| q.id), Some(2));
            view.set_turn_in_flight();
            view.cancel_turn(cx);
            assert!(!view.has_pending_question());
        })
        .unwrap();
    assert_eq!(answered(&mut answers[0]), Some(QuestionAnswer::Decline));
    assert_eq!(answered(&mut answers[1]), Some(QuestionAnswer::Cancel));
}

/// In a single-select form, picking after typing sends the pick alone; the
/// typed text does not ride along as a contradicting second answer.
#[gpui::test]
fn a_pick_after_typing_sends_only_the_pick(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(cx, vec![(1, color_question())]);
    window
        .update(cx, |view, live_window, cx| {
            view.open_question_text(1, "question_0_custom".into(), live_window, cx);
            let input = view
                .questions()
                .first()
                .and_then(|q| q.text("question_0_custom"))
                .cloned()
                .expect("the box is open");
            input.update(cx, |input, cx| input.set_value("green", live_window, cx));
            view.pick_question_option(1, "question_0".into(), "red".into(), cx);
            assert!(!view.has_pending_question());
        })
        .unwrap();
    let mut expected = std::collections::BTreeMap::new();
    expected.insert("question_0".into(), QuestionValue::Text("red".into()));
    assert_eq!(
        answered(&mut answers[0]),
        Some(QuestionAnswer::Submit(expected))
    );
}

/// Opening a box twice keeps the one that is shown rather than focusing a
/// second, unrendered one.
#[gpui::test]
fn opening_a_box_again_keeps_the_open_one(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(cx, vec![(1, color_question())]);
    window
        .update(cx, |view, live_window, cx| {
            let open = |view: &crate::view::AgentChatView| {
                view.questions()
                    .first()
                    .and_then(|q| q.text("question_0_custom"))
                    .map(|input| input.entity_id())
            };
            view.open_question_text(1, "question_0_custom".into(), live_window, cx);
            let first = open(view).expect("the box is open");
            view.open_question_text(1, "question_0_custom".into(), live_window, cx);
            assert_eq!(open(view), Some(first));
            // The test window has no widget-library root, which an input
            // needs to paint; leave nothing open for the next frame.
            view.skip_question(1, cx);
        })
        .unwrap();
}
