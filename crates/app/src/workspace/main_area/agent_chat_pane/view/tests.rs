use super::super::fold::FoldKey;
use super::super::rows::RowKind;

fn assistant_text_item(text: &str) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::AssistantText {
        text: text.to_string(),
        streaming: false,
        message_id: None,
    }
}

/// Build a minimal, offline `AgentChatView` (no cwd → `Idle` status, no
/// adapter spawned) as its own window root, so the post-turn marker
/// methods (`&mut self` + `Instant`/`Duration`, no `Workspace`) can be
/// driven directly. Lighter than `workspace/tests/agent_chat.rs`'s
/// `make_activity_view` (which goes through a full `Workspace` +
/// `create_agent_chat_pane`): `AgentChatView::new` only needs a
/// `Context<Self>`, not a `Workspace` at all.
fn make_test_view(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<super::AgentChatView> {
    crate::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        super::AgentChatView::new(
            0,
            window.window_handle(),
            None,
            super::AgentSessionStatus::Idle,
            None,
            None,
            "claude".to_string(),
            "Claude".to_string(),
            None,
            cx,
        )
    })
}

/// Appending a follow-up prompt must not make the previously visible agent
/// response collapse out from under the user. The model still keeps every
/// item either way; this pins the row projection so the prior response's
/// process prose remains visible after the new `UserText` anchor is added.
#[gpui::test]
fn echo_prompt_preserves_visible_tail_response(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items.push(daruda_acp::ChatItem::UserText("q1".into()));
            view.items.push(assistant_text_item("process"));
            view.items.push(assistant_text_item("final"));
            view.rebuild_rows();

            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::AgentItem(1)) && !r.hidden),
                "tail response starts expanded"
            );

            view.echo_prompt("q2".to_string(), cx);

            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::AgentItem(1)) && !r.hidden),
                "submitting the next prompt must not hide prior agent prose"
            );
            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::ConclusionItem(2)) && !r.hidden),
                "the prior conclusion remains visible too"
            );
        })
        .unwrap();
}

/// The preservation hook only freezes a response that is currently visible;
/// a user collapse remains authoritative across the next prompt.
#[gpui::test]
fn echo_prompt_respects_collapsed_tail_response(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.items.push(daruda_acp::ChatItem::UserText("q1".into()));
            view.items.push(assistant_text_item("process"));
            view.items.push(assistant_text_item("final"));
            view.toggle_fold(FoldKey::Response(0), cx);

            view.echo_prompt("q2".to_string(), cx);

            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::AgentItem(1)) && r.hidden),
                "an explicitly collapsed response stays collapsed after the next prompt"
            );
            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::ConclusionItem(2)) && !r.hidden),
                "the conclusion still surfaces from the collapsed response"
            );
        })
        .unwrap();
}

fn queued(id: u64, text: &str) -> super::QueuedPrompt {
    super::QueuedPrompt {
        id: super::PromptId(id),
        text: text.to_string(),
        origin: super::PromptOrigin::InApp,
    }
}

fn texts(prompts: &[super::QueuedPrompt]) -> Vec<String> {
    prompts.iter().map(|q| q.text.clone()).collect()
}

/// Stop must NOT discard the queue. A turn in flight with prompts buffered
/// behind it: `cancel_turn` settles the turn and moves the whole live queue
/// into the parked queue (preserved), leaving the live queue empty.
#[gpui::test]
fn cancel_turn_parks_the_queue_instead_of_clearing(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
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
        })
        .unwrap();
}

/// Resuming a parked queue moves the parked prompts back to the FRONT of the
/// live queue (they were submitted before anything queued after the Stop).
/// Offline (no handle) the pump can't dispatch, so they simply land in the
/// live queue in order — the reorder is what this pins.
#[gpui::test]
fn resume_queue_moves_parked_to_front_of_live(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.queue.paused_prompts.push(queued(1, "p1"));
            view.queue.paused_prompts.push(queued(2, "p2"));
            // A prompt queued (behind a since-cancelled turn) after the Stop.
            view.queue.pending_prompts.push(queued(3, "live"));

            view.resume_queue(cx);

            assert!(
                view.queue.paused_prompts.is_empty(),
                "resume drains the parked queue"
            );
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec!["p1".to_string(), "p2".to_string(), "live".to_string()],
                "parked prompts resume ahead of the live queue (FIFO)"
            );
        })
        .unwrap();
}

/// The per-item remove (×) and clear-all reach parked prompts too — the
/// strip renders those affordances on parked rows, so they must act on
/// `paused_prompts`, not just the live queue.
#[gpui::test]
fn remove_and_clear_reach_parked_prompts(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.queue.paused_prompts.push(queued(1, "a"));
            view.queue.paused_prompts.push(queued(2, "b"));
            view.queue.pending_prompts.push(queued(3, "c"));

            view.remove_queued(super::PromptId(1), cx);
            assert_eq!(
                texts(&view.queue.paused_prompts),
                vec!["b".to_string()],
                "removing a parked prompt drops it from the parked queue"
            );
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec!["c".to_string()],
                "the live queue is untouched by removing a parked prompt"
            );

            view.clear_queue(cx);
            assert!(
                view.queue.paused_prompts.is_empty() && view.queue.pending_prompts.is_empty(),
                "clear-all empties both the parked and live queues"
            );
        })
        .unwrap();
}

/// Editing a parked prompt then sending replaces it in place (order kept),
/// rather than falling through and enqueuing a duplicate as a new prompt.
#[gpui::test]
fn editing_a_parked_prompt_replaces_it_in_place(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.queue.paused_prompts.push(queued(1, "old"));
            view.begin_edit(super::PromptId(1), cx);

            view.send_prompt_text("new".to_string(), cx);

            assert_eq!(
                texts(&view.queue.paused_prompts),
                vec!["new".to_string()],
                "the parked prompt's text is replaced in place"
            );
            assert!(
                view.queue.pending_prompts.is_empty(),
                "editing does not enqueue a new prompt"
            );
            assert_eq!(view.queue.editing_prompt, None, "the edit flag is cleared");
        })
        .unwrap();
}

#[gpui::test]
fn start_telegram_watch_if_arms_only_for_telegram_origin(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            view.start_telegram_watch_if(super::PromptOrigin::InApp);
            assert!(
                !view.is_waiting_for_telegram_first_response(),
                "an in-app dispatch never arms the watch"
            );

            view.start_telegram_watch_if(super::PromptOrigin::Telegram);
            assert!(
                view.is_waiting_for_telegram_first_response(),
                "a telegram-origin dispatch arms the watch"
            );
        })
        .unwrap();
}

/// A telegram-origin prompt that has to wait behind an in-flight turn must
/// not arm the watch while merely queued — only once it actually drains
/// onto the wire.
#[gpui::test]
fn queued_telegram_prompt_arms_the_watch_only_once_drained(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            view.enqueue_prompt("from telegram".to_string(), super::PromptOrigin::Telegram);
            assert!(
                !view.is_waiting_for_telegram_first_response(),
                "queuing alone must not arm the watch"
            );

            view.set_turn_idle();
            view.drain_next_queued_prompt_for_test(cx);
            assert!(
                view.is_waiting_for_telegram_first_response(),
                "draining the telegram-origin prompt arms the watch"
            );
        })
        .unwrap();
}

#[gpui::test]
fn telegram_prompt_does_not_replace_in_app_queue_edit(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            let id = view.enqueue_prompt("local draft".to_string(), super::PromptOrigin::InApp);
            view.begin_edit(id, cx);

            let dispatch = view.send_prompt_text_for_telegram("from phone".to_string(), cx);

            assert_eq!(dispatch, super::PromptDispatch::Queued);
            assert_eq!(
                view.queue
                    .pending_prompts
                    .iter()
                    .map(|q| q.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["local draft", "from phone"],
                "a phone reply is a new prompt, not an edit commit"
            );
            assert_eq!(
                view.queue.editing_prompt,
                Some(id),
                "the in-app edit target remains active"
            );
        })
        .unwrap();
}

#[gpui::test]
fn queued_in_app_prompt_never_arms_the_watch(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            view.enqueue_prompt("typed in app".to_string(), super::PromptOrigin::InApp);
            view.set_turn_idle();

            view.drain_next_queued_prompt_for_test(cx);
            assert!(
                !view.is_waiting_for_telegram_first_response(),
                "an in-app-origin drain must not arm the watch"
            );
        })
        .unwrap();
}

#[gpui::test]
fn turn_end_resolves_telegram_watch_after_finalizing_streaming_text(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            view.start_telegram_first_response_watch_for_test(std::time::Instant::now());
            view.items.push(daruda_acp::ChatItem::AssistantText {
                text: "done".to_string(),
                streaming: true,
                message_id: None,
            });

            let effect = view.apply_event(
                daruda_acp::AcpEvent::TurnEnded {
                    completed_normally: true,
                    stop_reason: "EndTurn".into(),
                },
                "",
                false,
                cx,
            );

            assert_eq!(
                effect,
                super::TelegramFirstResponseEffect::Relay(super::FirstResponseOutcome::Text(
                    "done".to_string()
                ))
            );
            assert!(
                !view.is_waiting_for_telegram_first_response(),
                "the resolved watch is cleared at the terminal boundary"
            );
        })
        .unwrap();
}

/// The "Esc twice clears the queue" gesture must not be blocked by trailing
/// background-subagent liveness: after a Stop parks the queue, a running
/// subagent keeps `is_busy()` true (its quiescence window), but `handle_escape`
/// still clears the parked queue rather than routing back into a cancel.
#[gpui::test]
fn escape_clears_parked_queue_even_while_a_subagent_runs(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            // A queue parked by a prior Stop.
            view.queue.paused_prompts.push(queued(1, "a"));
            // A live child tool under a subagent parent → `is_busy()` true
            // even though no foreground turn is in flight.
            view.items.push(ChatItem::ToolCall(ToolCallItem {
                id: "child".into(),
                title: "child".into(),
                kind: ToolKindView::Read,
                tool_name: None,
                status: ToolStatusView::InProgress,
                diffs: Vec::new(),
                output: Vec::new(),
                raw_input: None,
                parent_tool_id: Some("parent".into()),
            }));
            assert!(view.is_busy(), "the trailing subagent keeps the pane busy");
            assert!(view.turn_is_idle(), "but no foreground turn is in flight");

            let outcome = view.handle_escape(cx);

            assert!(
                matches!(outcome, super::EscapeOutcome::ClearedQueue),
                "Escape clears the parked queue instead of re-cancelling"
            );
            assert!(
                view.queue.paused_prompts.is_empty(),
                "the parked queue is discarded"
            );
        })
        .unwrap();
}

/// `reconcile_post_turn` withholds the delta until `quiescence` has
/// elapsed since the dirty stamp, then relays it and advances the marker
/// so a second call (still no new text) is a no-op.
#[gpui::test]
fn reconcile_post_turn_waits_for_quiescence_then_dedups(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    let quiescence = std::time::Duration::from_millis(100);
    let dirty_at = std::time::Instant::now();

    window
        .update(cx, |view, _window, _cx| {
            view.items.push(assistant_text_item("done"));
            view.activity.post_turn_dirty_at = Some(dirty_at);
        })
        .unwrap();

    window
        .update(cx, |view, _window, _cx| {
            assert_eq!(
                view.reconcile_post_turn(dirty_at, quiescence),
                None,
                "not yet quiesced"
            );
        })
        .unwrap();

    let settled = dirty_at + quiescence + std::time::Duration::from_millis(1);
    window
        .update(cx, |view, _window, _cx| {
            assert_eq!(
                view.reconcile_post_turn(settled, quiescence),
                Some("done".to_string()),
                "quiesced: relays the delta and advances the marker"
            );
            assert_eq!(
                view.reconcile_post_turn(settled, quiescence),
                None,
                "second call has nothing new to relay"
            );
        })
        .unwrap();
}

/// `take_pending_post_turn` force-flushes a not-yet-quiesced follow-up
/// exactly once, then reports nothing pending.
#[gpui::test]
fn take_pending_post_turn_flushes_once(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            view.items.push(assistant_text_item("flushed"));
            view.activity.post_turn_dirty_at = Some(std::time::Instant::now());
            assert_eq!(view.take_pending_post_turn(), Some("flushed".to_string()));
            assert_eq!(view.take_pending_post_turn(), None);
        })
        .unwrap();
}

/// After `snap_post_turn_baseline()` syncs the marker to a pre-populated
/// history (as every `restoring = false` site does), a stray post-turn
/// update that arrives with no new assistant text must not resurrect the
/// replayed conversation as a "background follow-up".
#[gpui::test]
fn snap_post_turn_baseline_prevents_replay_from_being_relayed(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, _cx| {
            view.items.push(assistant_text_item("history-1"));
            view.items.push(assistant_text_item("history-2"));
            view.snap_post_turn_baseline();
            assert_eq!(view.activity.post_turn_relayed_assistant_texts, 2);

            view.activity.post_turn_dirty_at =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(10));
            assert_eq!(
                view.reconcile_post_turn(std::time::Instant::now(), super::POST_TURN_QUIESCENCE),
                None,
                "resumed baseline must not re-relay replayed history"
            );
        })
        .unwrap();
}

#[test]
fn post_turn_delta_none_when_nothing_new() {
    let items = vec![assistant_text_item("promise")];
    assert_eq!(super::post_turn_delta(&items, 1), None);
}

#[test]
fn post_turn_delta_returns_new_item_and_advances_count() {
    let items = vec![
        assistant_text_item("promise"),
        assistant_text_item("완료되었습니다"),
    ];
    assert_eq!(
        super::post_turn_delta(&items, 1),
        Some(("완료되었습니다".to_string(), 2))
    );
}

#[test]
fn post_turn_delta_joins_multiple_new_items() {
    let items = vec![
        assistant_text_item("a"),
        assistant_text_item("b"),
        assistant_text_item("c"),
    ];
    assert_eq!(
        super::post_turn_delta(&items, 1),
        Some(("b\n\nc".to_string(), 3))
    );
}

#[test]
fn post_turn_delta_ignores_whitespace_only_delta() {
    let items = vec![assistant_text_item("x"), assistant_text_item("   ")];
    assert_eq!(super::post_turn_delta(&items, 1), None);
}
