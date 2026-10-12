//! Stop and every other end of a run: what it parks, keeps and marks in the
//! transcript, and the items it has to settle.

use super::super::AgentChatView;
use super::*;
use crate::agent_chat_helpers::Rollup;
use crate::rows::LiveSubagentUnits;
use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, TextContent};
use daruda_acp::{AcpEvent, PlanEntryView, PlanPriority, PlanStatus, SessionUpdate, ToolCall};

/// Stop must preserve queued work and completion bookkeeping across the local
/// cancel paths.
#[gpui::test]
fn cancel_turn_parks_queue_preserves_completion_and_buffers_reprompt(
    cx: &mut gpui::TestAppContext,
) {
    use daruda_acp::ToolStatusView;

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            let reset = |view: &mut crate::view::AgentChatView| {
                view.queue = Default::default();
                view.items.clear();
                view.activity = Default::default();
            };

            view.set_turn_in_flight();
            view.queue.pending_prompts.push(queued(1, "a"));
            view.queue.pending_prompts.push(queued(2, "b"));

            view.cancel_turn(cx);

            assert!(view.turn_is_idle(), "Stop settles the turn locally");
            assert!(
                view.queue.pending_prompts.is_empty(),
                "the live queue is emptied by the Stop"
            );
            assert_eq!(
                texts(&view.queue.paused_prompts),
                vec!["a".to_string(), "b".to_string()],
                "Stop parks the queue (FIFO) instead of dropping it"
            );

            reset(view);
            view.set_turn_idle();
            view.items = vec![tool_call(
                "child",
                ToolStatusView::InProgress,
                Some("parent"),
            )];
            view.activity.pending_completion = Some(crate::view::TurnOutcome::Completed);
            assert!(
                view.is_busy(),
                "a running child subagent keeps the pane busy"
            );

            view.cancel_turn(cx);
            assert_eq!(
                view.activity.pending_completion,
                Some(crate::view::TurnOutcome::Completed),
                "Stop with no foreground turn keeps the already-captured completion"
            );

            reset(view);
            view.set_turn_in_flight();
            view.cancel_turn(cx);
            assert!(view.turn_is_idle(), "Stop settles the live turn locally");
            assert_eq!(
                view.activity.pending_completion,
                Some(crate::view::TurnOutcome::Stopped),
                "Stop of a live turn stashes Stopped"
            );

            reset(view);
            view.set_turn_in_flight();
            view.cancel_turn(cx);
            assert!(
                view.turn_awaiting_cancel_ack(),
                "the cancel window stays open until the ack"
            );

            view.send_prompt_text("again".into(), cx);
            assert!(
                view.turn_is_idle(),
                "the re-prompt is buffered, not raced onto the wire, during cancel"
            );
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec!["again".to_string()],
                "the re-prompt buffers client-side"
            );

            view.cancel_turn(cx);
            assert!(
                view.queue.pending_prompts.is_empty(),
                "the second Stop empties the live queue"
            );
            assert_eq!(
                texts(&view.queue.paused_prompts),
                vec!["again".to_string()],
                "the re-prompt is parked, not dropped"
            );

            view.apply_event(
                daruda_acp::AcpEvent::TurnEnded {
                    completed_normally: false,
                    stop_reason: "Cancelled".into(),
                    usage: None,
                },
                "",
                false,
                cx,
            );
            assert!(
                !view.turn_awaiting_cancel_ack(),
                "the ack closes the cancel window"
            );
            assert!(view.turn_is_idle(), "nothing left to run after both Stops");
        })
        .unwrap();
}

/// Stop leaves the same marker the adapter replays on `session/load`, so a live
/// pane and a restored one show the run cut in the same place. Gated on a live
/// turn: the SDK only writes its interrupt entry when a request was in flight,
/// and a marker the replay cannot reproduce would diverge on restore.
#[gpui::test]
fn stop_marks_the_transcript_only_when_a_turn_was_in_flight(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items = vec![assistant_text_item("working on it")];
            view.set_turn_in_flight();
            view.cancel_turn(cx);
            assert_eq!(
                view.items.last(),
                Some(&daruda_acp::ChatItem::Interrupted),
                "a stopped turn is marked where it was cut"
            );

            // Idle already (the trailing-subagent Stop): nothing was
            // interrupted, so the transcript gains nothing.
            let before = view.items.len();
            view.cancel_turn(cx);
            assert_eq!(
                view.items.len(),
                before,
                "a stop with no live turn adds no marker"
            );
        })
        .expect("view update");
}

/// A chunk already on the wire when Stop is pressed lands *after* the marker,
/// because nothing gates `AcpEvent::Update` on the open cancel window and the
/// adapter's stream path has no cancelled check either. Left there it reads as
/// a promptless turn under "Stopped by user" — and the SDK persists the
/// assistant message *before* its interrupt entry, so a restored pane would
/// order the same conversation the other way. The cancel ack is the point
/// after which no more chunks for that turn can arrive, so the marker settles
/// to the end there.
#[gpui::test]
fn late_content_does_not_strand_the_stop_marker_above_it(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items = vec![daruda_acp::ChatItem::UserText("go".into())];
            view.set_turn_in_flight();
            view.cancel_turn(cx);
            assert_eq!(view.items.last(), Some(&daruda_acp::ChatItem::Interrupted));

            // A chunk that was already in flight when the cancel went out.
            view.items.push(assistant_text_item("half a sentence"));
            view.apply_event(
                daruda_acp::AcpEvent::TurnEnded {
                    completed_normally: false,
                    stop_reason: "Cancelled".into(),
                    usage: None,
                },
                "",
                false,
                cx,
            );

            assert_eq!(
                view.items.last(),
                Some(&daruda_acp::ChatItem::Interrupted),
                "the marker settles below the text that beat it"
            );
            assert_eq!(
                view.items
                    .iter()
                    .filter(|i| matches!(i, daruda_acp::ChatItem::Interrupted))
                    .count(),
                1,
                "moved, not duplicated"
            );
        })
        .expect("view update");
}

// A run that can no longer change must stop reading as live.
//
// The rollup glyph pulses off `Rollup::Running`, which is derived from the
// items themselves (a `streaming` flag, a non-terminal tool status) — while the
// pulse pump only repaints panes `is_busy()` says are working. So an item left
// flagged live on an idle pane gives a dot nothing drives: frozen until an
// unrelated repaint samples the free-running clock, which reads as an erratic
// blink. Every path that ends a run has to settle its items.

fn chunk(text: &str, message_id: &str) -> AcpEvent {
    AcpEvent::Update(Box::new(SessionUpdate::AgentMessageChunk(
        ContentChunk::new(ContentBlock::Text(TextContent::new(text.to_string())))
            .message_id(message_id),
    )))
}

/// A plan the agent left mid-step, as a `PlanChanged` the pane folds in.
fn plan_with_a_live_step() -> AcpEvent {
    let entry = |content: &str, status| PlanEntryView {
        content: content.to_string(),
        priority: PlanPriority::Medium,
        status,
    };
    AcpEvent::PlanChanged(vec![
        entry("read the file", PlanStatus::Completed),
        entry("edit the file", PlanStatus::InProgress),
        entry("run the tests", PlanStatus::Pending),
    ])
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
    // The plan is a second live-flagged store behind the same pulse: an entry
    // left `InProgress` drives the header dot and the row glyph exactly as a
    // streaming item drives the rollup.
    assert!(
        !view.plan.iter().any(|e| e.status == PlanStatus::InProgress),
        "{what}: an idle pane's plan must not read in-progress ({:?})",
        view.plan
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
            view.apply_event(plan_with_a_live_step(), "", false, cx);
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
                    usage: None,
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
                    usage: None,
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
                view.apply_event(plan_with_a_live_step(), "", false, cx);

                drive(view, cx);
                assert_settled(view, what);
            })
            .unwrap();
    }
}

/// Text that arrives once the turn is over is transcript, not an errand.
///
/// The pane keeps it and stays at rest: nothing is owed to a phone, so nothing
/// holds the pulse open on a conversation the user already watched settle.
#[gpui::test]
async fn text_arriving_after_the_turn_ended_leaves_the_pane_at_rest(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.apply_event(chunk("background job finished", "m1"), "", false, cx);

            assert!(
                !view.maybe_active(),
                "a settled pane must not be woken by text it merely received"
            );
        })
        .unwrap();
}
