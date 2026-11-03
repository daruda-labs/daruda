//! An agent's question relayed to the phone: one card per field, answered
//! there field by field, and told when the desktop answered first.

use futures::{FutureExt as _, StreamExt as _};

use super::tests::{expect_ping, make_window};
use crate::remote_channel::bridge::{
    BotQuestionOutcome, PingPrompt, QuestionButton, QuestionPrompt, QuestionPromptRef,
    QuestionReply,
};
use crate::remote_channel::cards::tests::EditCapture;
use crate::surface::strings as s;
use crate::telegram::bridge::{BridgePing, TelegramTail};
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_acp::{QuestionField, QuestionFieldKind, QuestionOption, QuestionRequest};
use daruda_chat::view::QuestionEnd;

type Outbox = crate::telegram::global::TestOutbox;

/// A paired bridge that sends whatever the presence gate would, and one agent
/// chat pane.
fn bridged_pane(cx: &mut gpui::TestAppContext) -> (Outbox, gpui::Entity<super::Workspace>, PaneId) {
    let outbound = cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    config.telegram.only_when_away = false;
    cx.update(crate::app_presence::init);
    let (handle, workspace) = make_window(cx, &config);
    let pane = handle
        .update(cx, |_, window, cx| {
            workspace.update(cx, |ws, cx| ws.open_agent_chat_pane_for_test(window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    (outbound, workspace, pane)
}

fn option(value: &str) -> QuestionOption {
    QuestionOption {
        value: value.into(),
        label: value.to_uppercase(),
        description: None,
    }
}

fn field(key: &str, title: &str, kind: QuestionFieldKind) -> QuestionField {
    QuestionField {
        key: key.into(),
        title: Some(title.into()),
        description: None,
        required: false,
        kind,
        companion_of: None,
        default: None,
    }
}

/// Two questions: a required single select, then an optional multi-select.
fn form() -> QuestionRequest {
    let mut request = QuestionRequest {
        message: "Before I ship".into(),
        tool_call_id: Some("toolu_q".into()),
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![
            field(
                "question_0",
                "Which check?",
                QuestionFieldKind::SingleSelect(vec![option("clippy"), option("test")]),
            ),
            field(
                "question_1",
                "Which docs?",
                QuestionFieldKind::MultiSelect(vec![option("readme"), option("agents")]),
            ),
        ],
    };
    request.fields[0].required = true;
    request
}

fn ask(
    ws: &mut super::Workspace,
    pane: PaneId,
    request: QuestionRequest,
    cx: &mut gpui::Context<super::Workspace>,
) {
    let view = ws.agent_chat_view(pane).cloned().expect("view present");
    view.update(cx, |v, cx| v.ask_question_for_test(1, request, cx));
}

fn sent(outbox: &mut Outbox) -> Option<BridgePing> {
    outbox.next().now_or_never().flatten().map(expect_ping)
}

fn question_card(ping: &BridgePing) -> &QuestionPrompt {
    match &ping.prompt {
        Some(PingPrompt::Question(prompt)) => prompt,
        other => panic!("expected a question card, got {other:?}"),
    }
}

fn tail_text(ping: &BridgePing) -> &str {
    match &ping.tail {
        TelegramTail::Plain(text) => text,
        TelegramTail::Markdown(_) => panic!("a question card is plain: the agent's words verbatim"),
    }
}

fn labels(prompt: &QuestionPrompt) -> Vec<Vec<String>> {
    prompt
        .rows
        .iter()
        .map(|row| row.iter().map(|(label, _)| label.clone()).collect())
        .collect()
}

fn tap(
    ws: &mut super::Workspace,
    pane: PaneId,
    card: QuestionPromptRef,
    button: QuestionButton,
    cx: &mut gpui::Context<super::Workspace>,
) -> BotQuestionOutcome {
    ws.respond_bot_question(pane, card, QuestionReply::Button(button), cx)
}

/// The sweep sends each open question once, on its focused field; the next
/// field goes out only after the phone answered the one before.
#[gpui::test]
async fn a_question_is_relayed_once_per_field(cx: &mut gpui::TestAppContext) {
    let (mut outbox, workspace, pane) = bridged_pane(cx);
    workspace.update(cx, |ws, cx| {
        ask(ws, pane, form(), cx);
        ws.relay_outstanding_permissions(cx);
        let first = sent(&mut outbox).expect("the question goes out");
        assert_eq!(question_card(&first).question.field, 0);
        ws.relay_outstanding_permissions(cx);
        assert!(sent(&mut outbox).is_none(), "told once, not every tick");

        let outcome = tap(
            ws,
            pane,
            question_card(&first).question,
            QuestionButton::Option(0),
            cx,
        );
        assert_eq!(outcome, BotQuestionOutcome::Next);
        let second = sent(&mut outbox).expect("the next field goes out at once");
        assert_eq!(question_card(&second).question.field, 1);
        ws.relay_outstanding_permissions(cx);
        assert!(sent(&mut outbox).is_none());
    });
}

/// A tapped choice answers its field, and the card for the next one says
/// which question of the form it is. A required field's card has no skip of
/// its own, only "Skip all".
#[gpui::test]
async fn a_phone_option_answers_and_relays_the_next_field(cx: &mut gpui::TestAppContext) {
    let (mut outbox, workspace, pane) = bridged_pane(cx);
    workspace.update(cx, |ws, cx| {
        ask(ws, pane, form(), cx);
        ws.relay_outstanding_permissions(cx);
        let first = sent(&mut outbox).expect("the question goes out");
        let card = question_card(&first);
        assert_eq!(
            labels(card),
            [
                vec!["CLIPPY".to_owned()],
                vec!["TEST".to_owned()],
                vec![s::notification::telegram_question_skip()],
            ]
        );
        assert!(tail_text(&first).contains("Which check?"));
        assert_eq!(
            tap(ws, pane, card.question, QuestionButton::Option(1), cx),
            BotQuestionOutcome::Next
        );
        let second = sent(&mut outbox).expect("the next field");
        assert!(tail_text(&second).contains("Which docs?"));
        let agent = ws
            .agent_chat_view(pane)
            .unwrap()
            .read(cx)
            .agent_name()
            .to_owned();
        assert!(
            tail_text(&second).contains(&s::notification::telegram_question_asks_position(
                &agent, 2, 2
            ))
        );
        let view = ws.agent_chat_view(pane).cloned().unwrap();
        assert!(view.read(cx).has_pending_question(), "one field still open");
    });
}

/// The desktop answered first: the phone's card is edited to say so, and a
/// late tap on it is refused rather than reaching the agent a second time.
#[gpui::test]
async fn a_late_phone_answer_is_rejected_after_a_desktop_answer(cx: &mut gpui::TestAppContext) {
    let (mut outbox, workspace, pane) = bridged_pane(cx);
    cx.update(|cx| cx.set_global(EditCapture::default()));
    let view = workspace.update(cx, |ws, cx| {
        ask(ws, pane, form(), cx);
        ws.relay_outstanding_permissions(cx);
        ws.agent_chat_view(pane).cloned().unwrap()
    });
    deliver(&mut outbox, 7, cx);
    view.update(cx, |v, cx| v.skip_question(1, cx));
    cx.run_until_parked();
    cx.update(|cx| {
        let edits = &cx.global::<EditCapture>().edits;
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].label, s::notification::telegram_question_skipped());
    });
    workspace.update(cx, |ws, cx| {
        let card = QuestionPromptRef {
            session: view.read(cx).permission_session(),
            question_id: 1,
            field: 0,
        };
        assert_eq!(
            tap(ws, pane, card, QuestionButton::Option(0), cx),
            BotQuestionOutcome::AlreadySettled
        );
        assert!(sent(&mut outbox).is_none(), "nothing more goes out");
    });

    // An answer, not a skip, says the desktop gave it.
    workspace.update(cx, |ws, cx| {
        let mut single = form();
        single.fields.truncate(1);
        single.tool_call_id = Some("toolu_r".into());
        view.update(cx, |v, cx| v.ask_question_for_test(2, single, cx));
        ws.relay_outstanding_permissions(cx);
    });
    deliver(&mut outbox, 8, cx);
    view.update(cx, |v, cx| v.pick_question_option(2, 0, 0, cx));
    cx.run_until_parked();
    cx.update(|cx| {
        let edits = &cx.global::<EditCapture>().edits;
        assert_eq!(
            edits.last().map(|edit| edit.label.clone()),
            Some(s::notification::telegram_question_answered_on_desktop())
        );
    });
}

/// Deliver the card queued last as message `message_id`, so the ledger holds
/// a receipt a settlement can edit.
fn deliver(outbox: &mut Outbox, message_id: i64, cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let finish = outbox.begin_send(message_id, cx).expect("a queued card");
        finish(cx);
    });
}

/// The card names who is asking: the project and worktree, and the agent.
#[gpui::test]
async fn the_phone_message_says_who_asks(cx: &mut gpui::TestAppContext) {
    let (mut outbox, workspace, pane) = bridged_pane(cx);
    workspace.update(cx, |ws, cx| {
        ask(ws, pane, form(), cx);
        ws.relay_outstanding_permissions(cx);
        let ping = sent(&mut outbox).expect("the question goes out");
        let lane = ws.lane_ref_for_pane(pane).expect("the pane sits in a lane");
        let place = s::control::lane_path(
            ws.control_project_name(lane.project),
            ws.control_lane_name(lane),
        );
        let agent = ws
            .agent_chat_view(pane)
            .unwrap()
            .read(cx)
            .agent_name()
            .to_owned();
        assert!(ping.header.contains(&place), "{}", ping.header);
        assert!(ping.header.contains(&agent), "{}", ping.header);
        let tail = tail_text(&ping);
        assert!(
            tail.contains(&s::notification::telegram_question_asks(&agent)),
            "{tail}"
        );
        assert!(tail.contains("Before I ship"), "{tail}");
    });
}

/// A multi-select on the phone: each toggle re-sends the card with its
/// choices marked, and Send ends the form.
#[gpui::test]
async fn a_multi_select_is_toggled_and_sent_from_the_phone(cx: &mut gpui::TestAppContext) {
    let (mut outbox, workspace, pane) = bridged_pane(cx);
    workspace.update(cx, |ws, cx| {
        ask(ws, pane, form(), cx);
        ws.relay_outstanding_permissions(cx);
        let first = sent(&mut outbox).unwrap();
        tap(
            ws,
            pane,
            question_card(&first).question,
            QuestionButton::Option(0),
            cx,
        );
        let multi = sent(&mut outbox).unwrap();
        let unpicked = s::notification::telegram_question_unpicked;
        let picked = s::notification::telegram_question_picked;
        let send = s::notification::telegram_question_send();
        let skip_one = s::notification::telegram_question_skip_one();
        let skip = s::notification::telegram_question_skip();
        assert_eq!(
            labels(question_card(&multi)),
            [
                vec![unpicked("README")],
                vec![unpicked("AGENTS")],
                vec![send.clone(), skip_one.clone(), skip.clone()],
            ]
        );
        assert_eq!(
            tap(
                ws,
                pane,
                question_card(&multi).question,
                QuestionButton::Option(0),
                cx
            ),
            BotQuestionOutcome::Updated,
            "a toggle stays on its field"
        );
        let toggled = sent(&mut outbox).unwrap();
        assert_eq!(
            tap(
                ws,
                pane,
                question_card(&toggled).question,
                QuestionButton::Option(1),
                cx
            ),
            BotQuestionOutcome::Updated
        );
        let both = sent(&mut outbox).unwrap();
        assert_eq!(
            labels(question_card(&both)),
            [
                vec![picked("README")],
                vec![picked("AGENTS")],
                vec![send, skip_one, skip],
            ]
        );
        assert_eq!(
            tap(
                ws,
                pane,
                question_card(&both).question,
                QuestionButton::Send,
                cx
            ),
            BotQuestionOutcome::Submitted
        );
        let view = ws.agent_chat_view(pane).cloned().unwrap();
        assert!(!view.read(cx).has_pending_question());
        assert_eq!(
            view.read(cx).question_end_for_test("toolu_q"),
            Some(QuestionEnd::Answered)
        );
    });
}

/// "Skip all" on the phone declines the question and records it skipped.
#[gpui::test]
async fn skipping_from_the_phone_records_skipped(cx: &mut gpui::TestAppContext) {
    let (mut outbox, workspace, pane) = bridged_pane(cx);
    workspace.update(cx, |ws, cx| {
        ask(ws, pane, form(), cx);
        ws.relay_outstanding_permissions(cx);
        let first = sent(&mut outbox).unwrap();
        assert_eq!(
            tap(
                ws,
                pane,
                question_card(&first).question,
                QuestionButton::Skip,
                cx
            ),
            BotQuestionOutcome::Skipped
        );
        let view = ws.agent_chat_view(pane).cloned().unwrap();
        assert_eq!(
            view.read(cx).question_end_for_test("toolu_q"),
            Some(QuestionEnd::Skipped)
        );
        // A text reply to the settled card is refused, not sent as a prompt.
        assert_eq!(
            ws.respond_bot_question(
                pane,
                QuestionPromptRef {
                    session: view.read(cx).permission_session(),
                    question_id: 1,
                    field: 0,
                },
                QuestionReply::Text("late".into()),
                cx
            ),
            BotQuestionOutcome::AlreadySettled
        );
    });
}

/// "Skip this one" leaves an optional text field blank: the form goes with
/// the answers already given, not declined.
#[gpui::test]
async fn skipping_one_optional_field_from_the_phone_keeps_earlier_answers(
    cx: &mut gpui::TestAppContext,
) {
    let (mut outbox, workspace, pane) = bridged_pane(cx);
    workspace.update(cx, |ws, cx| {
        let mut request = form();
        request.fields[1] = field("question_1", "Anything else?", QuestionFieldKind::Text);
        ask(ws, pane, request, cx);
        ws.relay_outstanding_permissions(cx);
        let first = sent(&mut outbox).unwrap();
        tap(
            ws,
            pane,
            question_card(&first).question,
            QuestionButton::Option(1),
            cx,
        );
        let text = sent(&mut outbox).unwrap();
        assert_eq!(
            labels(question_card(&text)),
            [vec![
                s::notification::telegram_question_skip_one(),
                s::notification::telegram_question_skip(),
            ]]
        );
        assert_eq!(
            tap(
                ws,
                pane,
                question_card(&text).question,
                QuestionButton::SkipField,
                cx
            ),
            BotQuestionOutcome::Submitted
        );
        let view = ws.agent_chat_view(pane).cloned().unwrap();
        assert_eq!(
            view.read(cx).question_end_for_test("toolu_q"),
            Some(QuestionEnd::Answered)
        );
    });
}

#[gpui::test]
async fn a_task_pane_awaiting_an_answer_reports_waiting_on_answer(cx: &mut gpui::TestAppContext) {
    use crate::runs::result::{RunKey, RunState, WaitingOn};
    use crate::runs::{
        RunsHost,
        seed::{self, SeedRun},
    };
    let (_outbox, workspace, pane) = bridged_pane(cx);
    workspace.update(cx, |ws, cx| {
        let run = SeedRun::chat("/work");
        let id = seed::task(
            seed::spec(Default::default(), "question task"),
            Some((&run, seed::SeedEnd::Running)),
            cx,
        );
        let key = RunKey::Task(run.exec(&id));
        RunsHost::bind_execution(ws.run_pane(pane), run.exec(&id), cx);
        ask(ws, pane, form(), cx);
        ws.relay_outstanding_permissions(cx);
        assert!(matches!(
            RunsHost::run_query(key, cx).read().state,
            RunState::Running {
                waiting_on: Some(WaitingOn::Answer { ref question }),
                ..
            } if question == "Which check?"
        ));
    });
}
