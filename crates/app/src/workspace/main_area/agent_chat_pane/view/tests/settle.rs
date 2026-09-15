//! A run that can no longer change must stop reading as live.
//!
//! The rollup glyph pulses off `Rollup::Running`, which is derived from the
//! items themselves (a `streaming` flag, a non-terminal tool status) — while the
//! pulse pump only repaints panes `is_busy()` says are working. So an item left
//! flagged live on an idle pane gives a dot nothing drives: frozen until an
//! unrelated repaint samples the free-running clock, which reads as an erratic
//! blink. Every path that ends a run has to settle its items.

use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, TextContent};
use daruda_acp::{AcpEvent, SessionUpdate, ToolCall};

use super::super::AgentChatView;
use super::make_test_view;
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::Rollup;
use crate::workspace::main_area::agent_chat_pane::rows::LiveSubagentUnits;

fn chunk(text: &str, message_id: &str) -> AcpEvent {
    AcpEvent::Update(Box::new(SessionUpdate::AgentMessageChunk(
        ContentChunk::new(ContentBlock::Text(TextContent::new(text.to_string())))
            .message_id(message_id),
    )))
}

fn connected(session_id: &str) -> AcpEvent {
    AcpEvent::Connected {
        program: None,
        session_id: session_id.into(),
        modes: None,
        config_options: Vec::new(),
        capabilities: Default::default(),
        login_methods: Vec::new(),
    }
}

/// What the pane's own activity tracker says, next to what the transcript says.
/// The two disagreeing is the defect; asserting the pair keeps a fix that only
/// silenced the glyph from passing.
fn assert_settled(view: &AgentChatView, what: &str) {
    assert!(
        !view.is_busy(),
        "{what}: the pane must be idle for this assertion to mean anything"
    );
    let rollup = Rollup::of_kept_run(
        &view.items,
        0..view.items.len(),
        &LiveSubagentUnits::of(&view.items),
        |_| true,
    );
    assert_ne!(
        rollup,
        Rollup::Running,
        "{what}: an idle pane's run must not read Running ({:?})",
        view.items
    );
}

/// A `session/load` replays the prior conversation as `session/update`s and
/// closes with `Connected` — never a `TurnEnded`. Nothing else will arrive for
/// that conversation, so the gate closing is what has to settle it.
#[gpui::test]
fn a_finished_resume_settles_the_replayed_conversation(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.begin_connect(Some("sess-1".into()), cx);
            // A tool the prior process never finished, then the answer streamed
            // back in chunks — the shape a killed session leaves behind.
            view.apply_event(
                AcpEvent::Update(Box::new(SessionUpdate::ToolCall(ToolCall::new(
                    "t1", "Read",
                )))),
                "",
                false,
                cx,
            );
            view.apply_event(chunk("answer ", "m1"), "", false, cx);
            view.apply_event(chunk("done.", "m1"), "", false, cx);
            view.apply_event(connected("sess-1"), "", false, cx);

            assert_settled(view, "finished resume");
        })
        .unwrap();
}

/// A load the agent downgraded to a fresh `session/new` answers with a
/// different id. The replayed items stay in the transcript either way, so the
/// downgrade needs the same settle as a real resume.
#[gpui::test]
fn a_downgraded_resume_settles_what_it_replayed(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.begin_connect(Some("sess-1".into()), cx);
            view.apply_event(chunk("replayed", "m1"), "", false, cx);
            view.apply_event(connected("sess-2"), "", false, cx);

            assert_settled(view, "downgraded resume");
        })
        .unwrap();
}

/// An adapter that closes the event stream mid-load leaves no terminal event at
/// all; `abort_restore` is the gate's last exit and owes the same settle.
#[gpui::test]
fn an_aborted_restore_settles_what_it_projects(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.begin_connect(Some("sess-1".into()), cx);
            view.apply_event(chunk("half a reply", "m1"), "", false, cx);
            view.abort_restore(cx);

            assert_settled(view, "aborted restore");
        })
        .unwrap();
}

/// Stop settles the turn locally, but a chunk already on the wire lands after
/// that and starts a fresh streaming block. The cancel ack — the point after
/// which no further chunk for the cancelled turn can arrive — has to settle it.
#[gpui::test]
fn a_cancel_ack_settles_a_chunk_that_landed_after_the_stop(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            view.items = vec![daruda_acp::ChatItem::UserText("q".into())];
            view.cancel_turn(cx);
            view.apply_event(chunk("late", "m1"), "", false, cx);
            view.apply_event(
                AcpEvent::TurnEnded {
                    stop_reason: "Cancelled".into(),
                    completed_normally: false,
                },
                "",
                false,
                cx,
            );

            assert_settled(view, "cancel ack");
        })
        .unwrap();
}

/// The exits that were already correct, pinned as a table so the set is
/// readable in one place: a newly added exit is then a visible hole in this
/// list rather than a silent one. These pass today — they guard the list, they
/// do not drive it.
#[gpui::test]
fn every_turn_ending_exit_settles_its_items(cx: &mut gpui::TestAppContext) {
    type Exit = (
        &'static str,
        fn(&mut AgentChatView, &mut gpui::Context<AgentChatView>),
    );

    let exits: &[Exit] = &[
        ("TurnEnded", |view, cx| {
            view.apply_event(
                AcpEvent::TurnEnded {
                    stop_reason: "EndTurn".into(),
                    completed_normally: true,
                },
                "",
                false,
                cx,
            );
        }),
        ("TurnFailed", |view, cx| {
            view.apply_event(
                AcpEvent::TurnFailed(daruda_acp::AcpFailure::unclassified("prompt failed")),
                "",
                false,
                cx,
            );
        }),
        ("Error", |view, cx| {
            view.apply_event(
                AcpEvent::Error(daruda_acp::AcpFailure::unclassified("session died")),
                "",
                false,
                cx,
            );
        }),
        ("Stop", |view, cx| view.cancel_turn(cx)),
    ];

    for (what, drive) in exits {
        let window = make_test_view(cx);
        window
            .update(cx, |view, _window, cx| {
                view.set_turn_in_flight();
                view.items = vec![daruda_acp::ChatItem::UserText("q".into())];
                view.apply_event(
                    AcpEvent::Update(Box::new(SessionUpdate::ToolCall(ToolCall::new(
                        "t1", "Read",
                    )))),
                    "",
                    false,
                    cx,
                );
                view.apply_event(chunk("mid-flight", "m1"), "", false, cx);

                drive(view, cx);
                assert_settled(view, what);
            })
            .unwrap();
    }
}
