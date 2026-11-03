use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use daruda_acp::{
    AcpEvent, AcpSessionHandle, QuestionAnswer, QuestionField, QuestionFieldKind, QuestionOption,
    QuestionRequest, QuestionValue,
};

use super::{PhoneAnswerResult, PhoneReply};
use crate::view::tests::make_test_view;
use crate::view::{AgentChatEvent, AgentChatView, QuestionEnd};

fn option(value: &str) -> QuestionOption {
    QuestionOption {
        value: value.into(),
        label: format!("{value} label"),
        description: None,
    }
}

fn field(key: &str, kind: QuestionFieldKind, companion_of: Option<&str>) -> QuestionField {
    QuestionField {
        key: key.into(),
        title: Some(format!("{key}?")),
        description: None,
        required: false,
        kind,
        companion_of: companion_of.map(str::to_owned),
        default: None,
    }
}

/// A select with its "Other" box, a multi-select, then a yes/no.
fn form() -> QuestionRequest {
    QuestionRequest {
        message: "Several things".into(),
        tool_call_id: Some("toolu_1".into()),
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![
            field(
                "question_0",
                QuestionFieldKind::SingleSelect(vec![option("a"), option("b")]),
                None,
            ),
            field(
                "question_0_custom",
                QuestionFieldKind::Text,
                Some("question_0"),
            ),
            field(
                "question_1",
                QuestionFieldKind::MultiSelect(vec![option("x"), option("y"), option("z")]),
                None,
            ),
            field("question_2", QuestionFieldKind::Boolean, None),
        ],
    }
}

/// The settlements a view reported: question, how it ended, from the phone.
type Settled = Rc<RefCell<Vec<(u64, QuestionEnd, bool)>>>;

/// A view with a live session asked `request` as question 1, the receiver
/// its answer lands on, and the settlements it reports to the host.
fn asked(
    cx: &mut gpui::TestAppContext,
    request: QuestionRequest,
) -> (
    gpui::Entity<AgentChatView>,
    daruda_acp::QuestionProbe,
    Settled,
) {
    let window = make_test_view(cx);
    let view = window.root(cx).unwrap();
    let (handle, _probe) = AcpSessionHandle::detached_for_test();
    let answer = handle.park_question_for_test(1);
    let settled = Rc::new(RefCell::new(Vec::new()));
    let observed = settled.clone();
    cx.update(|cx| {
        cx.subscribe(&view, move |_, event: &AgentChatEvent, _| {
            if let AgentChatEvent::QuestionSettled {
                question_id,
                end,
                from_phone,
                ..
            } = event
            {
                observed
                    .borrow_mut()
                    .push((*question_id, *end, *from_phone));
            }
        })
        .detach();
    });
    view.update(cx, |view, cx| {
        view.status = crate::view::AgentSessionStatus::Connected;
        view.attach_handle(handle);
        view.composer_hint_empty = true;
        view.apply_event(
            AcpEvent::QuestionAsked {
                id: 1,
                question: request,
            },
            "",
            false,
            cx,
        );
    });
    (view, answer, settled)
}

/// The phone is offered each open question once, on the field the desktop
/// has focused, and offered it again only for a new recipient.
#[gpui::test]
fn a_question_is_offered_to_the_phone_once(cx: &mut gpui::TestAppContext) {
    let (view, _answer, _) = asked(cx, form());
    view.update(cx, |view, _| {
        let cards = view.take_questions_untold_to_phone();
        assert_eq!(cards.len(), 1);
        let card = &cards[0];
        assert_eq!((card.id, card.field, card.position), (1, 0, (1, 3)));
        assert_eq!(card.header, "question_0?");
        assert_eq!(card.message, "Several things");
        assert_eq!(card.options, ["a label", "b label"]);
        assert!(!card.multi && card.takes_text);
        assert_eq!(card.asked_by, view.agent_name());
        // Not marked: a relay that did not go out leaves it for the next sweep.
        let again = view.take_questions_untold_to_phone();
        assert_eq!(again.len(), 1);
        assert_ne!(again[0].step, card.step, "every card is a new step");
        view.mark_question_told_to_phone(1);
        assert!(view.take_questions_untold_to_phone().is_empty());
        view.forget_told_to_phone();
        assert_eq!(view.take_questions_untold_to_phone().len(), 1);
    });
}

/// A multi-select answered on the phone: each toggle hands back the same
/// field with its new picks, Send moves on, and the last field sends the
/// form. The desktop's focus and composer binding never move.
#[gpui::test]
fn a_multi_select_is_toggled_and_sent_from_the_phone(cx: &mut gpui::TestAppContext) {
    let (view, mut answer, settled) = asked(cx, form());
    let composer = view
        .read_with(cx, |view, _| view.question_composer())
        .expect("open");
    view.update(cx, |view, cx| {
        let PhoneAnswerResult::Next(card) =
            view.answer_question_from_phone(1, 0, PhoneReply::Option(1), cx)
        else {
            panic!("a pick on the first field moves on");
        };
        assert_eq!((card.field, card.multi), (1, true));
        assert_eq!(card.options, ["x label", "y label", "z label"]);
        let PhoneAnswerResult::Next(card) =
            view.answer_question_from_phone(1, 1, PhoneReply::Option(0), cx)
        else {
            panic!("a toggle stays on its field");
        };
        assert_eq!((card.field, card.picked.as_slice()), (1, &[0][..]));
        let PhoneAnswerResult::Next(card) =
            view.answer_question_from_phone(1, 1, PhoneReply::Option(2), cx)
        else {
            panic!("a toggle stays on its field");
        };
        assert_eq!(card.picked, [0, 2]);
        let PhoneAnswerResult::Next(card) =
            view.answer_question_from_phone(1, 1, PhoneReply::Send, cx)
        else {
            panic!("send moves on to the yes/no");
        };
        assert_eq!(card.field, 2);
        assert_eq!(card.options.len(), 2);
        let now = view.question_composer().expect("still open");
        assert_eq!(
            (now.field, now.bound, now.draft_became_answer),
            (composer.field, composer.bound, composer.draft_became_answer),
            "the desktop's focus and binding are untouched"
        );
        assert_eq!(
            view.answer_question_from_phone(1, 2, PhoneReply::Option(1), cx),
            PhoneAnswerResult::Submitted
        );
        assert!(!view.has_pending_question());
    });
    let expected = BTreeMap::from([
        ("question_0".to_owned(), QuestionValue::Text("b".into())),
        (
            "question_1".to_owned(),
            QuestionValue::Choices(vec!["x".into(), "z".into()]),
        ),
        ("question_2".to_owned(), QuestionValue::Bool(false)),
    ]);
    assert_eq!(answer.answer(), Some(QuestionAnswer::Submit(expected)));
    assert_eq!(*settled.borrow(), [(1, QuestionEnd::Answered, true)]);
}

/// Skip on the phone declines the question and records it skipped.
#[gpui::test]
fn skipping_from_the_phone_records_skipped(cx: &mut gpui::TestAppContext) {
    let (view, mut answer, settled) = asked(cx, form());
    view.update(cx, |view, cx| {
        assert_eq!(
            view.answer_question_from_phone(1, 0, PhoneReply::Skip, cx),
            PhoneAnswerResult::Skipped
        );
        assert_eq!(
            view.questions()
                .record_for_tool("toolu_1")
                .map(|r| r.outcome.clone()),
            Some(crate::view::questions::QuestionOutcome::Skipped)
        );
    });
    assert_eq!(answer.answer(), Some(QuestionAnswer::Decline));
    assert_eq!(*settled.borrow(), [(1, QuestionEnd::Skipped, true)]);
}

/// The first answer wins: once the desktop sent it, the phone's is refused
/// and nothing more reaches the agent.
#[gpui::test]
fn a_phone_answer_after_the_desktop_one_is_already_settled(cx: &mut gpui::TestAppContext) {
    let (view, mut answer, settled) = asked(cx, form());
    view.update(cx, |view, cx| {
        view.skip_question(1, cx);
        assert_eq!(
            view.answer_question_from_phone(1, 0, PhoneReply::Option(0), cx),
            PhoneAnswerResult::AlreadySettled
        );
    });
    assert_eq!(answer.answer(), Some(QuestionAnswer::Decline));
    assert_eq!(*settled.borrow(), [(1, QuestionEnd::Skipped, false)]);
}

/// A text reply answers a field that takes text and is refused by a yes/no,
/// which keeps the question as it was.
#[gpui::test]
fn text_from_the_phone_lands_only_where_text_belongs(cx: &mut gpui::TestAppContext) {
    let (view, _answer, _) = asked(cx, form());
    view.update(cx, |view, cx| {
        assert_eq!(
            view.answer_question_from_phone(1, 2, PhoneReply::Text("yes".into()), cx),
            PhoneAnswerResult::TextNotTaken
        );
        let PhoneAnswerResult::Next(card) =
            view.answer_question_from_phone(1, 0, PhoneReply::Text(" teal ".into()), cx)
        else {
            panic!("an Other answer moves on");
        };
        assert_eq!(card.field, 1);
        let question = view.questions().first().expect("still open");
        assert_eq!(question.draft.text("question_0_custom"), Some("teal"));
        assert_eq!(question.focus.field, 0, "desktop focus stays put");
    });
}

/// A select, then an optional text field.
fn select_then_text() -> QuestionRequest {
    QuestionRequest {
        message: "Two things".into(),
        tool_call_id: Some("toolu_1".into()),
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![
            field(
                "question_0",
                QuestionFieldKind::SingleSelect(vec![option("a"), option("b")]),
                None,
            ),
            field("question_1", QuestionFieldKind::Text, None),
        ],
    }
}

/// "Skip this one" leaves an optional field blank and moves on; with
/// nothing left the form goes with the answers already given.
#[gpui::test]
fn skipping_one_optional_field_keeps_the_other_answers(cx: &mut gpui::TestAppContext) {
    let (view, mut answer, settled) = asked(cx, select_then_text());
    view.update(cx, |view, cx| {
        let PhoneAnswerResult::Next(card) =
            view.answer_question_from_phone(1, 0, PhoneReply::Option(1), cx)
        else {
            panic!("the pick moves on");
        };
        assert_eq!(card.field, 1);
        assert!(card.optional);
        assert_eq!(
            view.answer_question_from_phone(1, 1, PhoneReply::SkipField, cx),
            PhoneAnswerResult::Submitted
        );
    });
    let expected = BTreeMap::from([("question_0".to_owned(), QuestionValue::Text("b".into()))]);
    assert_eq!(answer.answer(), Some(QuestionAnswer::Submit(expected)));
    assert_eq!(*settled.borrow(), [(1, QuestionEnd::Answered, true)]);
}

/// A field left blank is not offered again: leaving the first blank goes to
/// the second, and answering that one sends without coming back.
#[gpui::test]
fn a_field_left_blank_is_not_offered_again(cx: &mut gpui::TestAppContext) {
    let (view, mut answer, _) = asked(cx, select_then_text());
    view.update(cx, |view, cx| {
        let PhoneAnswerResult::Next(card) =
            view.answer_question_from_phone(1, 0, PhoneReply::SkipField, cx)
        else {
            panic!("moves on");
        };
        assert_eq!(card.field, 1);
        assert_eq!(
            view.answer_question_from_phone(1, 1, PhoneReply::Text("why".into()), cx),
            PhoneAnswerResult::Submitted
        );
    });
    let expected = BTreeMap::from([("question_1".to_owned(), QuestionValue::Text("why".into()))]);
    assert_eq!(answer.answer(), Some(QuestionAnswer::Submit(expected)));
}

/// A required field cannot be left blank on its own.
#[gpui::test]
fn a_required_field_has_no_skip_of_its_own(cx: &mut gpui::TestAppContext) {
    let mut request = select_then_text();
    request.fields[0].required = true;
    let (view, _answer, _) = asked(cx, request);
    view.update(cx, |view, cx| {
        let card = view.take_questions_untold_to_phone().remove(0);
        assert!(!card.optional);
        assert_eq!(
            view.answer_question_from_phone(1, 0, PhoneReply::SkipField, cx),
            PhoneAnswerResult::AlreadySettled
        );
        assert!(view.has_pending_question());
    });
}
