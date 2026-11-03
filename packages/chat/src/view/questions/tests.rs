use daruda_acp::{
    AcpEvent, AcpSessionHandle, QuestionAnswer, QuestionField, QuestionFieldKind, QuestionOption,
    QuestionRequest, QuestionValue,
};

use super::{ComposerBinding, QuestionOutcome};
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
        default: None,
    }
}

/// `AskUserQuestion`'s one-question shape: a select and its "Other" box.
fn color_question(tool: &str) -> QuestionRequest {
    QuestionRequest {
        message: "Which color?".into(),
        tool_call_id: Some(tool.into()),
        delivery: daruda_acp::QuestionDelivery::Blocking,
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

/// A select with its "Other" box, a multi-select, then a yes/no.
fn form_question(tool: &str) -> QuestionRequest {
    QuestionRequest {
        message: "Several things".into(),
        tool_call_id: Some(tool.into()),
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
                QuestionFieldKind::MultiSelect(vec![option("x"), option("y")]),
                None,
            ),
            field("question_2", QuestionFieldKind::Boolean, None),
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
    asked_over(cx, true, questions)
}

/// [`asked`], with the host's hint of whether the composer is empty.
fn asked_over(
    cx: &mut gpui::TestAppContext,
    composer_empty: bool,
    questions: Vec<(u64, QuestionRequest)>,
) -> (
    gpui::WindowHandle<crate::view::AgentChatView>,
    Vec<daruda_acp::QuestionProbe>,
) {
    asked_with(cx, composer_empty, None, questions)
}

/// [`asked`] on a pane whose questions may wait `timeout` untouched.
fn asked_timed(
    cx: &mut gpui::TestAppContext,
    timeout: std::time::Duration,
    questions: Vec<(u64, QuestionRequest)>,
) -> (
    gpui::WindowHandle<crate::view::AgentChatView>,
    Vec<daruda_acp::QuestionProbe>,
) {
    asked_with(cx, true, Some(timeout), questions)
}

fn asked_with(
    cx: &mut gpui::TestAppContext,
    composer_empty: bool,
    timeout: Option<std::time::Duration>,
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
            view.composer_hint_empty = composer_empty;
            view.set_question_timeout(timeout, cx);
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

fn outcome(view: &crate::view::AgentChatView, tool: &str) -> Option<QuestionOutcome> {
    view.questions()
        .record_for_tool(tool)
        .map(|r| r.outcome.clone())
}

/// A pending question is the pane's activity until it is answered; a
/// single-select form sends on the pick and leaves an answered record.
#[gpui::test]
fn picking_the_only_select_sends_and_leaves_an_answered_record(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(cx, vec![(1, color_question("toolu_1"))]);
    let mut expected = std::collections::BTreeMap::new();
    expected.insert("question_0".into(), QuestionValue::Text("blue".into()));
    window
        .update(cx, |view, _window, cx| {
            assert_eq!(view.activity_state(), ActivityState::AwaitingAnswer);
            assert_eq!(
                view.questions().pinned_tools().collect::<Vec<_>>(),
                ["toolu_1"]
            );
            view.pick_question_option(1, 0, 1, cx);
            assert!(!view.has_pending_question());
            assert_ne!(view.activity_state(), ActivityState::AwaitingAnswer);
            assert!(view.questions().open_for_tool("toolu_1").is_none());
            assert_eq!(
                outcome(view, "toolu_1"),
                Some(QuestionOutcome::Answered(expected.clone()))
            );
            assert_eq!(
                view.questions().recorded_tools().collect::<Vec<_>>(),
                ["toolu_1"]
            );
        })
        .unwrap();
    assert_eq!(
        answered(&mut answers[0]),
        Some(QuestionAnswer::Submit(expected))
    );
}

/// With an "Other" answer already committed, a pick no longer stands for
/// the whole answer: it waits for Send, which sends both.
#[gpui::test]
fn a_pick_after_committed_text_waits_for_send(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(cx, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.questions
                .get_mut(1)
                .expect("open")
                .draft
                .commit_text("question_0_custom", "green-ish");
            view.pick_question_option(1, 0, 0, cx);
            assert!(view.has_pending_question(), "the pick does not send");
            view.submit_question(1, cx);
            assert!(!view.has_pending_question());
        })
        .unwrap();
    let mut expected = std::collections::BTreeMap::new();
    expected.insert("question_0".into(), QuestionValue::Text("red".into()));
    expected.insert(
        "question_0_custom".into(),
        QuestionValue::Text("green-ish".into()),
    );
    assert_eq!(
        answered(&mut answers[0]),
        Some(QuestionAnswer::Submit(expected))
    );
}

/// A single-select pick moves on to the next unanswered field; a
/// multi-select pick toggles where it is.
#[gpui::test]
fn a_form_moves_focus_to_the_next_unanswered_field_after_a_pick(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(cx, vec![(1, form_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            let focus =
                |view: &crate::view::AgentChatView| view.questions().first().map(|q| q.focus.field);
            assert_eq!(focus(view), Some(0));
            view.pick_question_option(1, 0, 1, cx);
            assert!(view.has_pending_question(), "a form waits for Send");
            assert_eq!(focus(view), Some(1));
            view.pick_question_option(1, 1, 0, cx);
            assert_eq!(focus(view), Some(1), "a multi-select pick stays");
            let question = view.questions().first().expect("open");
            assert!(question.draft.is_picked("question_0", "b"));
            assert!(question.draft.is_picked("question_1", "x"));
        })
        .unwrap();
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
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![required, field("confirm", QuestionFieldKind::Boolean, None)],
    };
    let (window, mut answers) = asked(cx, vec![(1, question)]);
    window
        .update(cx, |view, _window, cx| {
            view.set_question_flag(1, 1, true, cx);
            view.submit_question(1, cx);
            assert!(view.has_pending_question(), "the required field is empty");
            view.pick_question_option(1, 0, 0, cx);
            view.pick_question_option(1, 0, 1, cx);
            view.pick_question_option(1, 0, 0, cx);
            assert!(
                view.has_pending_question(),
                "a pick does not send this form"
            );
            view.submit_question(1, cx);
            assert!(!view.has_pending_question());
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

/// Skip declines and records it; Stop cancels whatever is still open and
/// records each as cancelled.
#[gpui::test]
fn skip_records_skipped_and_stop_records_cancelled(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(
        cx,
        vec![
            (1, color_question("toolu_1")),
            (2, color_question("toolu_2")),
        ],
    );
    window
        .update(cx, |view, _window, cx| {
            assert_eq!(view.questions().len(), 2);
            assert_eq!(view.questions().first().map(|q| q.id), Some(1));
            view.skip_question(1, cx);
            assert_eq!(view.questions().first().map(|q| q.id), Some(2));
            assert_eq!(outcome(view, "toolu_1"), Some(QuestionOutcome::Skipped));
            view.set_turn_in_flight();
            view.cancel_turn(cx);
            assert!(!view.has_pending_question());
            assert_eq!(outcome(view, "toolu_2"), Some(QuestionOutcome::Cancelled));
        })
        .unwrap();
    assert_eq!(answered(&mut answers[0]), Some(QuestionAnswer::Decline));
    assert_eq!(answered(&mut answers[1]), Some(QuestionAnswer::Cancel));
}

/// The agent already settled a withdrawn question: it is recorded, and
/// nothing is sent back.
#[gpui::test]
fn a_withdrawn_question_records_withdrawn_without_replying(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(cx, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.apply_event(AcpEvent::QuestionWithdrawn { id: 1 }, "", false, cx);
            assert!(!view.has_pending_question());
            assert_eq!(outcome(view, "toolu_1"), Some(QuestionOutcome::Withdrawn));
        })
        .unwrap();
    assert_eq!(answered(&mut answers[0]), None);
}

/// A draft in the composer keeps the composer for prompts.
#[gpui::test]
fn a_question_arriving_over_a_draft_is_released(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked_over(cx, false, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, _cx| {
            let question = view.questions().first().expect("open");
            assert_eq!(question.composer, ComposerBinding::Released);
        })
        .unwrap();
}

/// An answer is text only, so attachments waiting to be sent keep the
/// composer for prompts.
#[gpui::test]
fn a_question_arriving_with_attachments_is_released(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.draft_attachments.push(daruda_acp::PromptAttachment {
                name: "notes.txt".into(),
                content: daruda_acp::AttachmentContent::ResourceLink {
                    uri: "urn:test:notes".into(),
                },
            });
            view.apply_event(
                AcpEvent::QuestionAsked {
                    id: 1,
                    question: color_question("toolu_1"),
                },
                "",
                false,
                cx,
            );
            let question = view.questions().first().expect("open");
            assert_eq!(question.composer, ComposerBinding::Released);
        })
        .unwrap();
}

/// The composer answers one question at a time: the oldest. The next takes
/// the binding once that one is settled.
#[gpui::test]
fn only_the_oldest_open_question_is_bound(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(
        cx,
        vec![
            (1, color_question("toolu_1")),
            (2, color_question("toolu_2")),
        ],
    );
    window
        .update(cx, |view, _window, cx| {
            let binding = |view: &crate::view::AgentChatView, tool: &str| {
                view.questions().open_for_tool(tool).map(|q| q.composer)
            };
            assert_eq!(
                binding(view, "toolu_1"),
                Some(ComposerBinding::Bound {
                    draft_became_answer: false
                })
            );
            assert_eq!(binding(view, "toolu_2"), Some(ComposerBinding::Released));
            view.skip_question(1, cx);
            assert_eq!(
                binding(view, "toolu_2"),
                Some(ComposerBinding::Bound {
                    draft_became_answer: false
                })
            );
        })
        .unwrap();
}

/// The composer summary counts the open questions behind the oldest.
#[gpui::test]
fn the_composer_summary_counts_questions_waiting_behind(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(
        cx,
        vec![
            (1, color_question("toolu_1")),
            (2, color_question("toolu_2")),
            (3, color_question("toolu_3")),
        ],
    );
    window
        .update(cx, |view, _window, cx| {
            let waiting = |view: &crate::view::AgentChatView| {
                view.question_composer()
                    .map(|c| (c.question_id, c.waiting_after))
            };
            assert_eq!(waiting(view), Some((1, 2)));
            view.skip_question(1, cx);
            assert_eq!(waiting(view), Some((2, 1)));
            view.skip_question(2, cx);
            assert_eq!(waiting(view), Some((3, 0)));
        })
        .unwrap();
}

/// Text that answers the oldest question leaves the composer empty, so the
/// next question takes it even though the hint still says "typed".
#[gpui::test]
fn a_typed_answer_passes_the_composer_to_the_next_question(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(
        cx,
        vec![
            (1, color_question("toolu_1")),
            (2, color_question("toolu_2")),
        ],
    );
    window
        .update(cx, |view, _window, cx| {
            view.set_composer_hint_empty(false, cx);
            assert!(view.answer_question_text(1, "teal".into(), cx));
            assert!(matches!(
                outcome(view, "toolu_1"),
                Some(QuestionOutcome::Answered(_))
            ));
            assert_eq!(
                view.questions()
                    .open_for_tool("toolu_2")
                    .map(|q| q.composer),
                Some(ComposerBinding::Bound {
                    draft_became_answer: false
                })
            );
        })
        .unwrap();
}

/// A second answer to a settled question (a late click, or Stop after a
/// pick) sends nothing more and keeps the first record.
#[gpui::test]
fn a_late_answer_after_resolution_is_a_no_op(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(cx, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.skip_question(1, cx);
            view.pick_question_option(1, 0, 0, cx);
            view.submit_question(1, cx);
            view.skip_question(1, cx);
            view.resolve_question(1, QuestionAnswer::Cancel, QuestionOutcome::Cancelled, cx);
            view.apply_event(AcpEvent::QuestionWithdrawn { id: 1 }, "", false, cx);
            view.cancel_pending_questions(cx);
            assert_eq!(outcome(view, "toolu_1"), Some(QuestionOutcome::Skipped));
        })
        .unwrap();
    assert_eq!(answered(&mut answers[0]), Some(QuestionAnswer::Decline));
}

/// `/clear` replaces the transcript the questions' rows belonged to: the open
/// one is cancelled with the agent, and no record outlives its row.
#[gpui::test]
fn clearing_the_session_cancels_open_questions_and_drops_records(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(
        cx,
        vec![
            (1, color_question("toolu_1")),
            (2, color_question("toolu_2")),
        ],
    );
    window
        .update(cx, |view, _window, cx| {
            view.skip_question(1, cx);
            assert!(outcome(view, "toolu_1").is_some());
            view.reset_for_new_session(cx);
            assert!(!view.has_pending_question());
            assert_eq!(outcome(view, "toolu_1"), None);
            assert_eq!(outcome(view, "toolu_2"), None);
            assert_eq!(view.questions().recorded_tools().count(), 0);
        })
        .unwrap();
    assert_eq!(answered(&mut answers[1]), Some(QuestionAnswer::Cancel));
}

fn call_under(id: &str, parent: Option<&str>) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::ToolCall(daruda_acp::ToolCallItem {
        id: id.to_owned(),
        title: format!("Tool {id}"),
        kind: daruda_acp::ToolKindView::Think,
        tool_name: None,
        status: daruda_acp::ToolStatusView::Completed,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        locations: Vec::new(),
        parent_tool_id: parent.map(str::to_owned),
        exit: None,
    })
}

/// A call nested in a subagent card renders inside its parent's row, so the
/// question resolves to that row: the fallback panel must not also show it, and
/// the ask must remeasure the parent card whose height it changes.
#[gpui::test]
fn a_nested_question_resolves_to_its_parents_row(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(cx, vec![]);
    window
        .update(cx, |view, _window, _cx| {
            view.items = vec![
                daruda_acp::ChatItem::UserText("q".into()),
                call_under("task", None),
                call_under("ask", Some("task")),
            ];
            view.rebuild_rows();
            let parent_row = crate::rows::item_row(&view.rows, 1).expect("parent owns a row");
            assert_eq!(view.question_row(Some("ask")), Some(parent_row));
            assert_eq!(view.question_row(Some("task")), Some(parent_row));
            assert_eq!(view.question_row(Some("missing")), None);
        })
        .unwrap();
}

/// The record folds as the question settles; after that the person's own
/// choice wins, including across later settles.
#[gpui::test]
fn a_resolved_question_folds_once_and_then_follows_the_person(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(
        cx,
        vec![
            (1, color_question("toolu_1")),
            (2, color_question("toolu_2")),
        ],
    );
    window
        .update(cx, |view, window, cx| {
            let first = crate::fold::FoldKey::Tool("toolu_1".into());
            let second = crate::fold::FoldKey::Tool("toolu_2".into());
            // A pending card is open whatever its fold state says; start both
            // open so only the settle can close them.
            view.set_fold_expanded(first.clone(), true, window, cx);
            view.set_fold_expanded(second.clone(), true, window, cx);
            view.skip_question(1, cx);
            assert!(!is_open(view, &first));
            view.toggle_fold(first.clone(), window, cx);
            assert!(is_open(view, &first));
            view.skip_question(2, cx);
            assert!(is_open(view, &first));
            assert!(!is_open(view, &second));
        })
        .unwrap();
}

/// Stop settles the open question too, and its record folds.
#[gpui::test]
fn a_cancelled_question_leaves_its_record_folded(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(cx, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, window, cx| {
            let key = crate::fold::FoldKey::Tool("toolu_1".into());
            view.set_fold_expanded(key, true, window, cx);
            view.set_turn_in_flight();
            view.cancel_turn(cx);
            assert_eq!(outcome(view, "toolu_1"), Some(QuestionOutcome::Cancelled));
            assert!(!is_open(
                view,
                &crate::fold::FoldKey::Tool("toolu_1".into())
            ));
        })
        .unwrap();
}

fn is_open(view: &crate::view::AgentChatView, key: &crate::fold::FoldKey) -> bool {
    let ctx = crate::agent_chat_helpers::fold_context(
        key,
        &view.items,
        &view.live_units,
        view.filter_matches.standalone_calls(),
    );
    view.fold.is_expanded(key, ctx)
}

/// A text field, then a select with its "Other" box: two questions.
fn name_and_color(tool: &str) -> QuestionRequest {
    QuestionRequest {
        message: "Who and which color?".into(),
        tool_call_id: Some(tool.into()),
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![
            field("name", QuestionFieldKind::Text, None),
            field(
                "color",
                QuestionFieldKind::SingleSelect(vec![option("red"), option("blue")]),
                None,
            ),
            field("color_custom", QuestionFieldKind::Text, Some("color")),
        ],
    }
}

fn focused_field(view: &crate::view::AgentChatView) -> Option<usize> {
    view.question_composer().map(|c| c.field)
}

/// Enter in the composer commits to the focused field and moves on. The
/// last field sends only once every question is answered; until then it
/// goes back to the one still open.
#[gpui::test]
fn answering_text_on_the_last_field_submits_once_everything_is_answered(
    cx: &mut gpui::TestAppContext,
) {
    let (window, mut answers) = asked(cx, vec![(1, name_and_color("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.bind_question_composer(1, Some(1), false, cx);
            view.answer_question_text(1, "teal".into(), cx);
            assert!(view.has_pending_question(), "the name is unanswered");
            assert_eq!(focused_field(view), Some(0));
            view.answer_question_text(1, "Ann".into(), cx);
            assert!(view.has_pending_question(), "only the last field sends");
            assert_eq!(focused_field(view), Some(1));
            view.answer_question_text(1, "teal".into(), cx);
            assert!(!view.has_pending_question());
        })
        .unwrap();
    let expected = std::collections::BTreeMap::from([
        ("name".to_owned(), QuestionValue::Text("Ann".into())),
        (
            "color_custom".to_owned(),
            QuestionValue::Text("teal".into()),
        ),
    ]);
    assert_eq!(
        answered(&mut answers[0]),
        Some(QuestionAnswer::Submit(expected))
    );
}

/// Committing without advancing stays on the field, as a field switch needs.
#[gpui::test]
fn committing_text_keeps_the_field_and_does_not_send(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(cx, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            assert!(view.commit_question_text(1, 0, "green".into(), cx));
            assert!(view.has_pending_question());
            assert_eq!(focused_field(view), Some(0));
            let question = view.questions().first().expect("open");
            assert_eq!(question.draft.text("question_0_custom"), Some("green"));
        })
        .unwrap();
}

/// A card pick can move focus on while the composer holds text typed for
/// the field before; the host names that field, so the text lands there.
#[gpui::test]
fn committing_names_the_field_the_text_was_typed_for(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(cx, vec![(1, form_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.pick_question_option(1, 0, 0, cx);
            assert_eq!(focused_field(view), Some(1));
            assert!(view.commit_question_text(1, 0, "typed for 0".into(), cx));
            assert_eq!(focused_field(view), Some(1), "committing does not move");
            let question = view.questions().first().expect("open");
            assert_eq!(
                question.draft.text("question_0_custom"),
                Some("typed for 0")
            );
        })
        .unwrap();
}

/// Text is refused where it cannot go: a yes/no field, or a question that
/// is not the oldest. The host keeps its composer as it is.
#[gpui::test]
fn text_for_a_yes_no_field_or_a_newer_question_is_refused(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(
        cx,
        vec![
            (1, form_question("toolu_1")),
            (2, color_question("toolu_2")),
        ],
    );
    window
        .update(cx, |view, _window, cx| {
            assert!(!view.commit_question_text(1, 2, "yes".into(), cx));
            view.bind_question_composer(1, Some(2), false, cx);
            assert!(!view.answer_question_text(1, "yes".into(), cx));
            assert_eq!(focused_field(view), Some(2), "nothing moved");
            assert!(!view.answer_question_text(2, "red-ish".into(), cx));
            assert!(!view.commit_question_text(2, 0, "red-ish".into(), cx));
            let newer = view.questions().open_for_tool("toolu_2").expect("open");
            assert_eq!(newer.draft.text("question_0_custom"), None);
            view.release_question_composer(2, cx);
            assert!(view.question_composer().expect("open").bound);
        })
        .unwrap();
}

/// A bind refused while attachments wait still consumes the card's
/// request, so a later bind cannot jump to that old field.
#[gpui::test]
fn a_refused_bind_drops_the_cards_request(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked_over(cx, false, vec![(1, name_and_color("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.request_question_composer(1, 1, cx);
            view.draft_attachments.push(daruda_acp::PromptAttachment {
                name: "notes.txt".into(),
                content: daruda_acp::AttachmentContent::ResourceLink {
                    uri: "urn:test:notes".into(),
                },
            });
            view.bind_question_composer(1, None, false, cx);
            assert!(!view.question_composer().expect("open").bound);
            view.draft_attachments.clear();
            view.bind_question_composer(1, None, false, cx);
            assert_eq!(focused_field(view), Some(0), "the old request is gone");
        })
        .unwrap();
}

/// Sending a prompt past the question declines it, and the record says the
/// person answered in conversation instead.
#[gpui::test]
fn skip_for_prompt_declines_and_records_skipped_for_prompt(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(cx, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.skip_question_for_prompt(1, cx);
            assert!(!view.has_pending_question());
            assert_eq!(
                outcome(view, "toolu_1"),
                Some(QuestionOutcome::SkippedForPrompt)
            );
        })
        .unwrap();
    assert_eq!(answered(&mut answers[0]), Some(QuestionAnswer::Decline));
}

/// Taking the composer over a prompt draft makes that draft the answer's
/// start, which the host says once: the first commit, a rebind, or a
/// release ends the hint.
#[gpui::test]
fn binding_over_a_draft_marks_draft_became_answer_once(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked_over(cx, false, vec![(1, name_and_color("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            let hint = |view: &crate::view::AgentChatView| {
                view.question_composer()
                    .map(|c| (c.bound, c.draft_became_answer))
            };
            assert_eq!(hint(view), Some((false, false)));
            view.bind_question_composer(1, None, true, cx);
            assert_eq!(hint(view), Some((true, true)));
            view.commit_question_text(1, 0, "my draft".into(), cx);
            assert_eq!(hint(view), Some((true, false)));
            view.bind_question_composer(1, Some(1), true, cx);
            assert_eq!(hint(view), Some((true, false)), "already bound");
            view.release_question_composer(1, cx);
            assert_eq!(hint(view), Some((false, false)));
            view.bind_question_composer(1, None, false, cx);
            assert_eq!(hint(view), Some((true, false)), "an empty composer");
        })
        .unwrap();
}

/// Only the oldest open question may hold the composer, even after the
/// person released it: a newer one waits its turn, and asking to bind it
/// does nothing.
#[gpui::test]
fn a_newer_question_never_binds_over_an_older_released_one(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(cx, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            let binding = |view: &crate::view::AgentChatView, tool: &str| {
                view.questions()
                    .open_for_tool(tool)
                    .map(|q| q.composer.is_bound())
            };
            view.release_question_composer(1, cx);
            view.apply_event(
                AcpEvent::QuestionAsked {
                    id: 2,
                    question: color_question("toolu_2"),
                },
                "",
                false,
                cx,
            );
            assert_eq!(binding(view, "toolu_2"), Some(false));
            view.bind_question_composer(2, None, false, cx);
            assert_eq!(binding(view, "toolu_2"), Some(false));
            let composer = view.question_composer().expect("open");
            assert_eq!((composer.question_id, composer.bound), (1, false));
        })
        .unwrap();
}

type ViewOp =
    dyn Fn(&mut crate::view::AgentChatView, &mut gpui::Context<crate::view::AgentChatView>);

/// The host resyncs its strip on one event per operation that changes the
/// composer summary, and hears nothing from one that does not.
#[gpui::test]
fn composer_summary_changes_are_announced_once_per_operation(cx: &mut gpui::TestAppContext) {
    use std::cell::Cell;
    use std::rc::Rc;

    let (window, _answers) = asked(cx, vec![]);
    let view = window.root(cx).expect("open");
    let heard = Rc::new(Cell::new(0));
    let count = heard.clone();
    cx.update(|cx| {
        cx.subscribe(&view, move |_, event, _| {
            if matches!(event, crate::view::AgentChatEvent::QuestionComposerChanged) {
                count.set(count.get() + 1);
            }
        })
        .detach();
    });
    let mut step = |label: &str, expected: usize, op: &ViewOp| {
        heard.set(0);
        view.update(cx, |view, cx| op(view, cx));
        assert_eq!(heard.get(), expected, "{label}");
    };
    step("ask", 1, &|view, cx| {
        view.apply_event(
            AcpEvent::QuestionAsked {
                id: 1,
                question: form_question("toolu_1"),
            },
            "",
            false,
            cx,
        );
    });
    // It changes the "+N waiting" count the strip shows.
    step("a later question", 1, &|view, cx| {
        view.apply_event(
            AcpEvent::QuestionAsked {
                id: 2,
                question: color_question("toolu_2"),
            },
            "",
            false,
            cx,
        );
    });
    step("pick moves focus", 1, &|view, cx| {
        view.pick_question_option(1, 0, 0, cx)
    });
    step("multi toggle", 0, &|view, cx| {
        view.pick_question_option(1, 1, 0, cx)
    });
    step("commit", 0, &|view, cx| {
        view.commit_question_text(1, 0, "x".into(), cx);
    });
    step("release", 1, &|view, cx| {
        view.release_question_composer(1, cx)
    });
    step("release again", 0, &|view, cx| {
        view.release_question_composer(1, cx)
    });
    step("bind", 1, &|view, cx| {
        view.bind_question_composer(1, Some(0), false, cx)
    });
    step("settle the oldest", 1, &|view, cx| {
        view.skip_question(1, cx)
    });
    step("withdraw the last", 1, &|view, cx| {
        view.apply_event(AcpEvent::QuestionWithdrawn { id: 2 }, "", false, cx);
    });
    step("nothing open", 0, &|view, cx| view.skip_question(2, cx));
}

/// "Answer in your own words" on the card: the view notes the field and
/// asks the host to focus the composer with what that field already holds;
/// the host's bind then lands on that field.
#[gpui::test]
fn writing_from_the_card_asks_the_host_to_focus_the_composer(cx: &mut gpui::TestAppContext) {
    use std::cell::RefCell;
    use std::rc::Rc;

    let (window, _answers) = asked_over(cx, false, vec![(1, name_and_color("toolu_1"))]);
    let view = window.root(cx).expect("open");
    let prefills = Rc::new(RefCell::new(Vec::new()));
    let seen = prefills.clone();
    cx.update(|cx| {
        cx.subscribe(&view, move |_, event, _| {
            if let crate::view::AgentChatEvent::FocusComposer {
                question_id,
                prefill,
            } = event
            {
                seen.borrow_mut().push((*question_id, prefill.clone()));
            }
        })
        .detach();
    });
    view.update(cx, |view, cx| {
        view.questions
            .get_mut(1)
            .expect("open")
            .draft
            .commit_text("color_custom", "teal");
        view.request_question_composer(1, 1, cx);
        assert_eq!(focused_field(view), Some(0), "the host commits first");
        view.bind_question_composer(1, None, true, cx);
        assert_eq!(focused_field(view), Some(1));
        assert!(view.question_composer().expect("open").draft_became_answer);
    });
    assert_eq!(*prefills.borrow(), [(1, Some("teal".to_owned()))]);
}

/// An answer is text only, so an explicit bind waits while attachments are
/// pending, as the bind on arrival does.
#[gpui::test]
fn binding_waits_while_attachments_are_pending(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked_over(cx, false, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.draft_attachments.push(daruda_acp::PromptAttachment {
                name: "notes.txt".into(),
                content: daruda_acp::AttachmentContent::ResourceLink {
                    uri: "urn:test:notes".into(),
                },
            });
            view.bind_question_composer(1, None, true, cx);
            assert!(!view.question_composer().expect("open").bound);
        })
        .unwrap();
}

fn running_call(id: &str) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::ToolCall(daruda_acp::ToolCallItem {
        id: id.to_owned(),
        title: format!("Tool {id}"),
        kind: daruda_acp::ToolKindView::Think,
        tool_name: None,
        status: daruda_acp::ToolStatusView::InProgress,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        locations: Vec::new(),
        parent_tool_id: None,
        exit: None,
    })
}

/// Only the oldest open question may take the composer, so only its write
/// row is a button; a newer card shows the same row as a plain hint.
#[gpui::test]
fn only_the_oldest_question_offers_a_write_button(cx: &mut gpui::TestAppContext) {
    let window = crate::view::tests::make_test_view(cx);
    cx.run_until_parked();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let view = window.root(&mut visual).unwrap();
    view.update(&mut visual, |view, cx| {
        view.items = vec![
            daruda_acp::ChatItem::UserText("q".into()),
            running_call("tool_a"),
            running_call("tool_b"),
        ];
        view.rebuild_rows();
        for (id, tool) in [(1, "tool_a"), (2, "tool_b")] {
            let question = name_and_color(tool);
            view.apply_event(
                daruda_acp::AcpEvent::QuestionAsked { id, question },
                "",
                false,
                cx,
            );
        }
        cx.notify();
    });
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds("agent-chat-question-1-1-write")
            .is_some()
    );
    assert!(
        visual
            .debug_bounds("agent-chat-question-card-tool_b")
            .is_some(),
        "the newer card paints"
    );
    assert!(
        visual
            .debug_bounds("agent-chat-question-2-1-write")
            .is_none()
    );
}

/// The host loads a field's committed text into the composer when that
/// field takes it; nothing for a field without text or a newer question.
#[gpui::test]
fn a_fields_committed_text_is_readable_for_the_composer(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(
        cx,
        vec![
            (1, name_and_color("toolu_1")),
            (2, color_question("toolu_2")),
        ],
    );
    window
        .update(cx, |view, _window, cx| {
            assert_eq!(view.question_field_text(1, 0), None);
            assert!(view.commit_question_text(1, 1, "teal".into(), cx));
            assert_eq!(view.question_field_text(1, 1).as_deref(), Some("teal"));
            assert_eq!(view.question_field_text(1, 0), None);
            assert_eq!(view.question_field_text(2, 0), None, "not the oldest");
        })
        .unwrap();
}

/// "↑ Question" brings the card's top into view.
#[gpui::test]
fn revealing_a_question_scrolls_to_its_row(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(cx, vec![]);
    window
        .update(cx, |view, _window, cx| {
            view.items = vec![
                daruda_acp::ChatItem::UserText("q".into()),
                running_call("tool_a"),
                daruda_acp::ChatItem::UserText("later".into()),
            ];
            view.rebuild_rows();
            view.apply_event(
                AcpEvent::QuestionAsked {
                    id: 1,
                    question: color_question("tool_a"),
                },
                "",
                false,
                cx,
            );
            let row = view.question_row(Some("tool_a")).expect("card row");
            view.list_state.scroll_to_end();
            view.reveal_question(1, cx);
            assert_eq!(view.list_state.logical_scroll_top().item_ix, row);
        })
        .unwrap();
}

fn highlighted_row(view: &crate::view::AgentChatView) -> Option<usize> {
    match view.questions().first()?.focus.highlight? {
        super::QuestionHighlight::Row(row) => Some(row),
        super::QuestionHighlight::Send => None,
    }
}

fn on_send(view: &crate::view::AgentChatView) -> bool {
    view.question_composer().is_some_and(|c| c.send_highlighted)
}

/// ↑↓ in an empty bound composer: the first press lands on the first row
/// (down) or the last (up), then moves one row at a time and stops at the
/// ends. The write row is the last row.
#[gpui::test]
fn rows_move_and_clamp_including_the_write_row(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(cx, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            assert_eq!(highlighted_row(view), None);
            view.move_question_row(1, false, cx);
            assert_eq!(highlighted_row(view), Some(2), "up starts on the write row");
            view.move_question_row(1, true, cx);
            assert_eq!(
                highlighted_row(view),
                Some(2),
                "down stops at the write row"
            );
            for expected in [1, 0, 0] {
                view.move_question_row(1, false, cx);
                assert_eq!(highlighted_row(view), Some(expected));
            }
        })
        .unwrap();
    let (window, _answers) = asked(cx, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.move_question_row(1, true, cx);
            assert_eq!(
                highlighted_row(view),
                Some(0),
                "down starts on the first row"
            );
        })
        .unwrap();
}

/// Digits are the card's row numbers: one past the options is the write
/// row, which asks for the composer instead of answering. A number with no
/// row, or any digit on a text field, stays a typed character.
#[gpui::test]
fn number_beyond_options_selects_the_write_row(cx: &mut gpui::TestAppContext) {
    use std::cell::RefCell;
    use std::rc::Rc;

    let (window, mut answers) = asked(cx, vec![(1, name_and_color("toolu_1"))]);
    let view = window.root(cx).expect("open");
    let focus_requests = Rc::new(RefCell::new(0));
    let seen = focus_requests.clone();
    cx.update(|cx| {
        cx.subscribe(&view, move |_, event, _| {
            if matches!(event, crate::view::AgentChatEvent::FocusComposer { .. }) {
                *seen.borrow_mut() += 1;
            }
        })
        .detach();
    });
    view.update(cx, |view, cx| {
        assert!(
            !view.choose_question_number(1, 1, cx),
            "a text field types its digits"
        );
        view.bind_question_composer(1, Some(1), false, cx);
        assert!(!view.choose_question_number(1, 4, cx), "no fourth row");
        assert_eq!(highlighted_row(view), None);
        assert!(view.choose_question_number(1, 3, cx));
        assert_eq!(highlighted_row(view), Some(2));
        assert_eq!(focused_field(view), Some(1), "the write row answers here");
        assert!(view.has_pending_question());
    });
    assert_eq!(*focus_requests.borrow(), 1);
    assert_eq!(answered(&mut answers[0]), None);
}

/// Space toggles the highlighted option of a multi-select only; elsewhere
/// it is a typed space.
#[gpui::test]
fn toggle_ignores_single_select(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(cx, vec![(1, form_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.move_question_row(1, true, cx);
            assert!(!view.toggle_question_row(1, cx));
            let question = view.questions().first().expect("open");
            assert!(!question.draft.is_picked("question_0", "a"));

            assert!(view.move_question_field(1, true, String::new(), cx));
            view.move_question_row(1, true, cx);
            assert!(view.toggle_question_row(1, cx));
            let question = view.questions().first().expect("open");
            assert!(question.draft.is_picked("question_1", "x"));
            assert_eq!(highlighted_row(view), Some(0), "a toggle stays put");
            assert!(view.toggle_question_row(1, cx));
            let question = view.questions().first().expect("open");
            assert!(!question.draft.is_picked("question_1", "x"));
        })
        .unwrap();
}

/// Tab and Shift+Tab move between fields. The composer's text was typed for
/// the field being left, so it is committed there first; a field that takes
/// no text refuses the move and keeps the text out of the next field.
#[gpui::test]
fn moving_fields_commits_the_composer_text_to_the_old_field(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(cx, vec![(1, form_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.move_question_row(1, true, cx);
            assert!(view.move_question_field(1, true, "teal".into(), cx));
            assert_eq!(focused_field(view), Some(1));
            assert_eq!(highlighted_row(view), None, "a new field starts unlit");
            assert_eq!(view.question_field_text(1, 0).as_deref(), Some("teal"));

            assert!(view.move_question_field(1, true, String::new(), cx));
            assert_eq!(focused_field(view), Some(2));
            assert!(
                view.move_question_field(1, true, String::new(), cx),
                "past the last field is send"
            );
            assert!(on_send(view));
            assert_eq!(focused_field(view), Some(2), "send keeps the last field");
            assert!(!view.move_question_field(1, true, String::new(), cx));
            assert!(view.move_question_field(1, false, String::new(), cx));
            assert!(!on_send(view), "Shift+Tab from send returns to the field");
            assert_eq!(focused_field(view), Some(2));
            assert!(
                !view.move_question_field(1, false, "hmm".into(), cx),
                "yes/no takes no text"
            );
            assert_eq!(focused_field(view), Some(2));

            assert!(view.move_question_field(1, false, String::new(), cx));
            assert!(view.move_question_field(1, false, String::new(), cx));
            assert_eq!(focused_field(view), Some(0));
            assert_eq!(
                view.question_field_text(1, 0).as_deref(),
                Some("teal"),
                "leaving with an empty composer keeps the committed text"
            );
            assert!(!view.move_question_field(1, false, String::new(), cx));
        })
        .unwrap();
}

/// Empty Enter: the highlighted row is chosen; with none highlighted, a
/// fully answered form sends and an unanswered one waits.
#[gpui::test]
fn choosing_picks_the_highlighted_row_or_sends_an_answered_form(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(cx, vec![(1, form_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.choose_question_row(1, cx);
            assert!(view.has_pending_question(), "nothing to choose yet");
            view.move_question_row(1, true, cx);
            view.move_question_row(1, true, cx);
            view.choose_question_row(1, cx);
            let question = view.questions().first().expect("open");
            assert!(question.draft.is_picked("question_0", "b"));
            assert_eq!(focused_field(view), Some(1), "a single pick moves on");

            view.move_question_row(1, true, cx);
            view.choose_question_row(1, cx);
            assert_eq!(focused_field(view), Some(1), "a multi pick stays");
            assert!(view.move_question_field(1, true, String::new(), cx));
            view.move_question_row(1, true, cx);
            view.choose_question_row(1, cx);
            assert!(view.has_pending_question());
            view.choose_question_row(1, cx);
            assert!(!view.has_pending_question(), "answered, so Enter sends");
        })
        .unwrap();
    let expected = std::collections::BTreeMap::from([
        ("question_0".to_owned(), QuestionValue::Text("b".into())),
        (
            "question_1".to_owned(),
            QuestionValue::Choices(vec!["x".into()]),
        ),
        ("question_2".to_owned(), QuestionValue::Bool(true)),
    ]);
    assert_eq!(
        answered(&mut answers[0]),
        Some(QuestionAnswer::Submit(expected))
    );
}

fn multi_only(tool: &str) -> QuestionRequest {
    QuestionRequest {
        message: "Which checks?".into(),
        tool_call_id: Some(tool.into()),
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![field(
            "checks",
            QuestionFieldKind::MultiSelect(vec![option("x"), option("y")]),
            None,
        )],
    }
}

/// Enter on a multi-select toggles, so a form that is one
/// multi-select is sent from the send button past its last row.
#[gpui::test]
fn a_lone_multi_select_is_answered_and_sent_by_keyboard(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(cx, vec![(1, multi_only("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.move_question_row(1, true, cx);
            view.choose_question_row(1, cx);
            assert!(view.has_pending_question(), "Enter on an option toggles");
            view.move_question_row(1, true, cx);
            view.move_question_row(1, true, cx);
            assert!(on_send(view), "past the last row is send");
            view.move_question_row(1, true, cx);
            assert!(on_send(view), "send is the end");
            view.move_question_row(1, false, cx);
            assert_eq!(highlighted_row(view), Some(1), "up returns to the rows");
            view.move_question_row(1, true, cx);
            view.choose_question_row(1, cx);
            assert!(!view.has_pending_question());
        })
        .unwrap();
    let expected = std::collections::BTreeMap::from([(
        "checks".to_owned(),
        QuestionValue::Choices(vec!["x".into()]),
    )]);
    assert_eq!(
        answered(&mut answers[0]),
        Some(QuestionAnswer::Submit(expected))
    );
}

/// The send button sends with an optional field left blank; Enter on it
/// takes the button's own gate, not "every field answered".
#[gpui::test]
fn a_form_with_an_optional_blank_is_sent_by_keyboard(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(cx, vec![(1, name_and_color("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            assert!(view.move_question_field(1, true, String::new(), cx));
            view.choose_question_number(1, 1, cx);
            assert_eq!(focused_field(view), Some(0), "the pick moves to the blank");
            view.choose_question_row(1, cx);
            assert!(
                view.has_pending_question(),
                "unlit Enter waits for every field"
            );
            assert!(view.move_question_field(1, true, String::new(), cx));
            assert!(view.move_question_field(1, true, String::new(), cx));
            assert!(on_send(view));
            view.choose_question_row(1, cx);
            assert!(!view.has_pending_question());
        })
        .unwrap();
    let expected =
        std::collections::BTreeMap::from([("color".to_owned(), QuestionValue::Text("red".into()))]);
    assert_eq!(
        answered(&mut answers[0]),
        Some(QuestionAnswer::Submit(expected))
    );
}

/// Tab on the last field commits what the composer holds to it before
/// lighting send.
#[gpui::test]
fn tab_at_the_last_field_commits_the_text_then_lands_on_send(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(cx, vec![(1, name_and_color("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.bind_question_composer(1, Some(1), false, cx);
            assert!(view.move_question_field(1, true, "teal".into(), cx));
            assert!(on_send(view));
            assert_eq!(view.question_field_text(1, 1).as_deref(), Some("teal"));
            view.clear_question_highlight(1, cx);
            assert!(!on_send(view), "typing leaves send for the field");
            assert_eq!(focused_field(view), Some(1));
        })
        .unwrap();
}

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// When question `id` was asked, the instant its countdown runs from.
fn asked_at(view: &crate::view::AgentChatView, id: u64) -> std::time::Instant {
    view.questions()
        .open()
        .find(|q| q.id == id)
        .expect("open")
        .asked_at
}

/// The deadline question `id` counts down to, if its countdown runs.
fn countdown(view: &crate::view::AgentChatView, id: u64) -> Option<std::time::Instant> {
    view.questions()
        .open()
        .find(|q| q.id == id)
        .and_then(|q| q.countdown())
}

/// An untouched question is declined once its time is up, and the
/// record says it expired and after how long. Later ticks change nothing.
#[gpui::test]
fn an_untouched_question_expires_once_with_decline(cx: &mut gpui::TestAppContext) {
    use std::cell::Cell;
    use std::rc::Rc;

    let (window, mut answers) = asked_timed(cx, TIMEOUT, vec![(1, color_question("toolu_1"))]);
    let view = window.root(cx).expect("open");
    let settled = Rc::new(Cell::new(0));
    let count = settled.clone();
    cx.update(|cx| {
        cx.subscribe(&view, move |_, event, _| {
            if matches!(event, crate::view::AgentChatEvent::QuestionSettled { .. }) {
                count.set(count.get() + 1);
            }
        })
        .detach();
    });
    view.update(cx, |view, cx| {
        let start = asked_at(view, 1);
        assert_eq!(countdown(view, 1), Some(start + TIMEOUT));
        view.tick_activity(start + TIMEOUT - std::time::Duration::from_millis(1), cx);
        assert!(view.has_pending_question(), "not yet");
        view.tick_activity(start + TIMEOUT, cx);
        assert!(!view.has_pending_question());
        assert_eq!(
            outcome(view, "toolu_1"),
            Some(QuestionOutcome::Expired { after_secs: 60 })
        );
        view.tick_activity(start + TIMEOUT * 2, cx);
        view.skip_question(1, cx);
        assert_eq!(
            outcome(view, "toolu_1"),
            Some(QuestionOutcome::Expired { after_secs: 60 })
        );
    });
    assert_eq!(answered(&mut answers[0]), Some(QuestionAnswer::Decline));
    assert_eq!(settled.get(), 1, "settled exactly once");
}

/// With no timeout the question waits however long it takes.
#[gpui::test]
fn without_a_timeout_a_question_never_expires(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked(cx, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            let start = asked_at(view, 1);
            assert_eq!(countdown(view, 1), None);
            view.tick_activity(start + TIMEOUT * 1000, cx);
            assert!(view.has_pending_question());
        })
        .unwrap();
}

/// Once the person acts on the question, by a pick on the card or by text
/// in the bound composer, the countdown stops and never restarts: not when
/// the composer empties again, not on a config reload.
#[gpui::test]
fn touching_the_question_stops_the_countdown_for_good(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked_timed(
        cx,
        TIMEOUT,
        vec![(1, form_question("toolu_1")), (2, form_question("toolu_2"))],
    );
    window
        .update(cx, |view, _window, cx| {
            let start = asked_at(view, 2);
            // A multi-select toggled on and off answers nothing, but is a touch.
            view.pick_question_option(1, 1, 0, cx);
            view.pick_question_option(1, 1, 0, cx);
            assert_eq!(countdown(view, 1), None);
            view.set_question_timeout(Some(TIMEOUT), cx);
            assert_eq!(countdown(view, 1), None, "a reload does not restart it");
            view.tick_activity(start + TIMEOUT * 10, cx);
            assert!(view.questions().get(1).is_some(), "the touched one waits");
            assert!(
                view.questions().get(2).is_none(),
                "the untouched one expired"
            );
        })
        .unwrap();

    let (window, _answers) = asked_timed(cx, TIMEOUT, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            let start = asked_at(view, 1);
            view.set_composer_hint_empty(false, cx);
            assert_eq!(countdown(view, 1), None, "typing in the bound composer");
            view.set_composer_hint_empty(true, cx);
            assert_eq!(countdown(view, 1), None, "emptying it does not restart");
            view.tick_activity(start + TIMEOUT * 10, cx);
            assert!(view.has_pending_question());
        })
        .unwrap();
}

/// A prompt draft in a released composer is not an answer, so the
/// question it waits beside keeps counting down.
#[gpui::test]
fn a_draft_in_a_released_composer_does_not_stop_the_countdown(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked_with(
        cx,
        false,
        Some(TIMEOUT),
        vec![(1, color_question("toolu_1"))],
    );
    window
        .update(cx, |view, _window, cx| {
            let start = asked_at(view, 1);
            assert!(!view.questions().get(1).expect("open").composer.is_bound());
            view.set_composer_hint_empty(false, cx);
            assert_eq!(countdown(view, 1), Some(start + TIMEOUT));
        })
        .unwrap();
}

/// Committing blank text changes no answer, so it is not a touch.
#[gpui::test]
fn a_blank_commit_does_not_stop_the_countdown(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked_timed(cx, TIMEOUT, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            let start = asked_at(view, 1);
            assert!(view.commit_question_text(1, 0, "   ".into(), cx));
            assert_eq!(countdown(view, 1), Some(start + TIMEOUT));
            assert!(view.commit_question_text(1, 0, "teal".into(), cx));
            assert_eq!(countdown(view, 1), None);
        })
        .unwrap();
}

/// The person's answer and the deadline settle through one path: whichever
/// comes first sends the only reply, and the other is a no-op.
#[gpui::test]
fn an_answer_racing_the_deadline_sends_exactly_one_reply(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked_timed(
        cx,
        TIMEOUT,
        vec![
            (1, color_question("toolu_1")),
            (2, color_question("toolu_2")),
        ],
    );
    let mut red = std::collections::BTreeMap::new();
    red.insert("question_0".into(), QuestionValue::Text("red".into()));
    window
        .update(cx, |view, _window, cx| {
            let start = asked_at(view, 2);
            // The answer lands first, past the deadline but before a tick.
            view.pick_question_option(1, 0, 0, cx);
            view.tick_activity(start + TIMEOUT, cx);
            assert_eq!(
                outcome(view, "toolu_1"),
                Some(QuestionOutcome::Answered(red.clone()))
            );
            // The deadline lands first; the late pick changes nothing.
            assert_eq!(
                outcome(view, "toolu_2"),
                Some(QuestionOutcome::Expired { after_secs: 60 })
            );
            view.pick_question_option(2, 0, 0, cx);
            assert_eq!(
                outcome(view, "toolu_2"),
                Some(QuestionOutcome::Expired { after_secs: 60 })
            );
        })
        .unwrap();
    assert_eq!(answered(&mut answers[0]), Some(QuestionAnswer::Submit(red)));
    assert_eq!(answered(&mut answers[1]), Some(QuestionAnswer::Decline));
}

/// A reload retimes an open question nobody touched, from when it was
/// asked; turning the timeout on counts from the reload instead, so a
/// question that already waited longer is not declined on the spot.
#[gpui::test]
fn a_reload_retimes_only_untouched_questions(cx: &mut gpui::TestAppContext) {
    let (window, _answers) = asked_timed(cx, TIMEOUT, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            let start = asked_at(view, 1);
            view.set_question_timeout(Some(TIMEOUT * 2), cx);
            assert_eq!(countdown(view, 1), Some(start + TIMEOUT * 2));
            view.set_question_timeout(None, cx);
            assert_eq!(countdown(view, 1), None);
            let before = std::time::Instant::now();
            view.set_question_timeout(Some(TIMEOUT), cx);
            let deadline = countdown(view, 1).expect("running again");
            assert!(deadline >= before + TIMEOUT, "counted from the reload");
        })
        .unwrap();
}

/// Working through the card from the keyboard, or asking for the
/// composer, is acting on the question: each stops its countdown.
#[gpui::test]
fn navigating_or_binding_stops_the_countdown(cx: &mut gpui::TestAppContext) {
    type Verb = fn(&mut crate::view::AgentChatView, &mut gpui::Context<crate::view::AgentChatView>);
    let verbs: [(&str, Verb); 3] = [
        ("arrow", |view, cx| view.move_question_row(1, true, cx)),
        ("tab", |view, cx| {
            assert!(view.move_question_field(1, true, String::new(), cx));
        }),
        ("bind", |view, cx| {
            view.release_question_composer(1, cx);
            view.bind_question_composer(1, None, false, cx);
        }),
    ];
    for (name, verb) in verbs {
        let (window, _answers) = asked_timed(cx, TIMEOUT, vec![(1, form_question("toolu_1"))]);
        window
            .update(cx, |view, _window, cx| {
                assert!(countdown(view, 1).is_some(), "{name}: running");
                verb(view, cx);
                assert_eq!(countdown(view, 1), None, "{name}: stopped");
            })
            .unwrap();
    }
}

/// Late desktop and phone replies cannot settle the next question after withdrawal.
#[gpui::test]
fn late_replies_after_withdrawal_leave_the_next_question_open(cx: &mut gpui::TestAppContext) {
    use crate::view::{PhoneAnswerResult, PhoneReply};

    let (window, mut answers) = asked(
        cx,
        vec![
            (1, color_question("toolu_1")),
            (2, color_question("toolu_2")),
        ],
    );
    window
        .update(cx, |view, _window, cx| {
            view.apply_event(AcpEvent::QuestionWithdrawn { id: 1 }, "", false, cx);
            view.pick_question_option(1, 0, 0, cx);
            view.submit_question_from_card(1, cx);
            assert_eq!(
                view.answer_question_from_phone(1, 0, PhoneReply::Option(0), cx),
                PhoneAnswerResult::AlreadySettled
            );
            assert_eq!(outcome(view, "toolu_1"), Some(QuestionOutcome::Withdrawn));
            assert_eq!(
                view.question_composer().expect("next question").question_id,
                2
            );
            assert_eq!(outcome(view, "toolu_2"), None);
        })
        .unwrap();
    assert_eq!(answered(&mut answers[0]), None);
    assert_eq!(answered(&mut answers[1]), None);
}

/// Message disclosure changes layout, not answers or keyboard focus.
#[gpui::test]
fn expanding_a_question_message_preserves_the_form_and_stops_expiry(cx: &mut gpui::TestAppContext) {
    let (window, mut answers) = asked(cx, vec![(1, color_question("toolu_1"))]);
    window
        .update(cx, |view, _window, cx| {
            view.set_question_timeout(Some(std::time::Duration::from_secs(60)), cx);
            let focus = view.questions().first().expect("open").focus;
            assert!(view.question_composer().expect("open").deadline.is_some());
            view.toggle_question_message(1, cx);
            let question = view.questions().first().expect("open");
            assert!(question.message_expanded);
            assert_eq!(question.focus, focus);
            assert!(question.draft.answer(&question.request).is_none());
            assert!(view.question_composer().expect("open").deadline.is_none());
            view.toggle_question_message(1, cx);
            assert!(!view.questions().first().expect("open").message_expanded);
        })
        .unwrap();
    assert_eq!(answered(&mut answers[0]), None);
}
