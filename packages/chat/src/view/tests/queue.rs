//! The prompt queue — parking, the resume gesture, edits, removal and the cap —
//! and the phone watch a prompt from Telegram arms when it leaves the queue.

use super::*;

/// The empty-composer Enter gesture: the first press arms, the second resumes.
/// The confirmation step is what keeps a stray Enter from firing the whole
/// parked queue, mirroring Esc-twice on the discard side. The parked prompts
/// go back ahead of anything queued after the Stop: they were submitted first.
#[gpui::test]
fn empty_submit_arms_then_resumes_parked_ahead_of_live(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.queue.paused_prompts.push(queued(1, "p1"));
            view.queue.paused_prompts.push(queued(2, "p2"));
            // A prompt queued (behind a since-cancelled turn) after the Stop.
            view.queue.pending_prompts.push(queued(3, "live"));
            assert!(!view.resume_armed(), "a parked queue starts unarmed");

            assert!(
                matches!(
                    view.handle_empty_submit(cx),
                    crate::view::EmptySubmitOutcome::Armed
                ),
                "the first empty Enter arms rather than resuming"
            );
            assert!(view.resume_armed(), "the strip now asks for a second Enter");
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec!["live".to_string()],
                "arming moves nothing — the queue is still parked"
            );

            assert!(
                matches!(
                    view.handle_empty_submit(cx),
                    crate::view::EmptySubmitOutcome::Resumed
                ),
                "the second empty Enter resumes"
            );
            assert!(
                view.queue.paused_prompts.is_empty(),
                "resume drains the parked queue"
            );
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec!["p1".to_string(), "p2".to_string(), "live".to_string()],
                "parked prompts resume ahead of the live queue (FIFO)"
            );
            assert!(!view.resume_armed(), "resuming leaves nothing armed");
        })
        .unwrap();
}

/// One Escape discards a parked queue whatever else holds the pane: an armed
/// resume gesture must not turn the discard into a three-press gesture, and a
/// trailing subagent — busy through its quiescence window, with no foreground
/// turn — must not route the press back into a cancel.
#[gpui::test]
fn escape_discards_a_parked_queue_in_one_press(cx: &mut gpui::TestAppContext) {
    #[derive(Debug, Clone, Copy)]
    enum Holding {
        ArmedGesture,
        TrailingSubagent,
    }
    for holding in [Holding::ArmedGesture, Holding::TrailingSubagent] {
        let window = make_test_view(cx);
        window
            .update(cx, |view, _window, cx| {
                // A queue parked by a prior Stop.
                view.queue.paused_prompts.push(queued(1, "a"));
                match holding {
                    Holding::ArmedGesture => {
                        view.handle_empty_submit(cx);
                        assert!(view.resume_armed());
                    }
                    Holding::TrailingSubagent => {
                        view.items.push(tool_call(
                            "child",
                            daruda_acp::ToolStatusView::InProgress,
                            Some("parent"),
                        ));
                        assert!(view.is_busy(), "the trailing subagent keeps the pane busy");
                        assert!(view.turn_is_idle(), "but no foreground turn is in flight");
                    }
                }

                assert!(
                    matches!(
                        view.handle_escape(cx),
                        crate::view::EscapeOutcome::ClearedQueue
                    ),
                    "{holding:?}: Escape clears the parked queue"
                );
                assert!(
                    view.queue.paused_prompts.is_empty(),
                    "{holding:?}: one Escape discards the parked queue"
                );
                assert!(!view.resume_armed(), "{holding:?}");
            })
            .unwrap();
    }
}

/// With nothing parked the gesture is inert: an empty Enter must not arm a
/// state the strip would then have no parked rows to explain.
#[gpui::test]
fn empty_submit_ignored_without_parked_prompts(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.queue.pending_prompts.push(queued(1, "live"));

            assert!(
                matches!(
                    view.handle_empty_submit(cx),
                    crate::view::EmptySubmitOutcome::Ignored
                ),
                "a live-only queue is not the parked queue the gesture resumes"
            );
            assert!(!view.resume_armed());
        })
        .unwrap();
}

/// Every way out of an armed resume gesture leaves the queue unarmed, so a
/// later park cannot land in a pre-armed queue that one Enter would fire. Stop
/// matters most: Telegram and macro prompts start turns without a composer
/// change to disarm. A composer change disarms too — the next Enter is a new
/// prompt, and the gesture starts over at arm.
#[gpui::test]
fn every_exit_from_an_armed_queue_leaves_it_unarmed(cx: &mut gpui::TestAppContext) {
    #[derive(Debug, Clone, Copy)]
    enum Exit {
        ComposerChange,
        ClearAll,
        Resume,
        Stop,
    }
    for exit in [
        Exit::ComposerChange,
        Exit::ClearAll,
        Exit::Resume,
        Exit::Stop,
    ] {
        let window = make_test_view(cx);
        window
            .update(cx, |view, _window, cx| {
                view.queue.paused_prompts.push(queued(1, "a"));
                view.handle_empty_submit(cx);
                assert!(view.resume_armed(), "{exit:?}");

                match exit {
                    Exit::ComposerChange => view.disarm_resume(cx),
                    Exit::ClearAll => view.clear_queue(cx),
                    Exit::Resume => view.resume_queue(cx),
                    Exit::Stop => {
                        view.set_turn_in_flight();
                        view.queue.pending_prompts.push(queued(2, "b"));
                        view.cancel_turn(cx);
                    }
                }
                view.queue.paused_prompts.push(queued(3, "parked later"));
                assert!(!view.resume_armed(), "{exit:?} left the queue armed");
                if let Exit::ComposerChange = exit {
                    assert!(
                        matches!(
                            view.handle_empty_submit(cx),
                            crate::view::EmptySubmitOutcome::Armed
                        ),
                        "after disarming, the gesture starts over at arm"
                    );
                }
            })
            .unwrap();
    }
}

/// The armed flag is only ever read through `resume_armed`, which also requires
/// a parked queue — so a queue emptied under an armed gesture can never leave a
/// second Enter pointing at nothing.
#[gpui::test]
fn resume_armed_requires_a_parked_queue(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.queue.paused_prompts.push(queued(1, "a"));
            view.handle_empty_submit(cx);
            assert!(view.resume_armed());

            view.remove_queued(crate::view::PromptId(1), cx);
            assert!(
                !view.resume_armed(),
                "removing the last parked row disarms without a separate clear"
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

            view.remove_queued(crate::view::PromptId(1), cx);
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

/// Editing a queued prompt replaces its slot in place, whether it is still live
/// or parked. If the edited row has already left the queue, sending falls
/// through to a brand-new queued prompt so the typed text is not lost.
#[gpui::test]
fn editing_prompt_replaces_live_or_parked_and_falls_through_when_stale(
    cx: &mut gpui::TestAppContext,
) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.send_prompt_text("first".into(), cx);
            view.send_prompt_text("second".into(), cx);
            view.send_prompt_text("third".into(), cx);
            let middle = view.queue.pending_prompts[1].id;
            view.begin_edit(middle, cx);
            assert_eq!(
                view.queue.editing_prompt,
                Some(middle),
                "begin_edit records the target"
            );

            view.send_prompt_text("edited".into(), cx);
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec![
                    "first".to_string(),
                    "edited".to_string(),
                    "third".to_string()
                ],
                "editing replaces the live slot in place, order preserved"
            );
            assert!(
                view.queue.editing_prompt.is_none(),
                "send clears the editing flag"
            );
            assert!(view.items.is_empty(), "an in-place edit does not echo");

            view.queue = Default::default();
            view.items.clear();
            view.queue.paused_prompts.push(queued(1, "old"));
            view.begin_edit(crate::view::PromptId(1), cx);

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

            view.queue = Default::default();
            view.items.clear();
            view.send_prompt_text("only".into(), cx);
            let id = view.queue.pending_prompts[0].id;
            view.begin_edit(id, cx);
            // Model the target leaving the queue while the composer still targets it.
            view.remove_queued(id, cx);
            assert_eq!(
                view.queue.editing_prompt,
                Some(id),
                "removing a row does not by itself clear the editing flag"
            );
            assert!(view.queue.pending_prompts.is_empty());

            view.send_prompt_text("typed".into(), cx);
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec!["typed".to_string()],
                "a drained edit target falls through to a new queued prompt"
            );
            assert!(
                view.queue.editing_prompt.is_none(),
                "the stale editing flag is cleared on send"
            );
            assert!(
                view.items.is_empty(),
                "no handle means the new prompt is queued, not echoed"
            );
        })
        .unwrap();
}

/// The cap lives on the queue itself so every producer meets it, and a
/// refusal is a named outcome — an unbounded queue and a silent drop are both
/// worse than being told. A parked prompt still has to drain, so it counts:
/// otherwise pressing Stop would reset the cap.
#[gpui::test]
fn a_full_queue_refuses_instead_of_growing_or_dropping(cx: &mut gpui::TestAppContext) {
    for parked in [false, true] {
        let window = make_test_view(cx);
        window
            .update(cx, |view, _window, cx| {
                if !parked {
                    view.set_turn_in_flight();
                }
                view.fill_queue_for_test(crate::QUEUE_DEPTH_MAX);
                if parked {
                    view.queue.paused_prompts = std::mem::take(&mut view.queue.pending_prompts);
                }
                assert_eq!(
                    view.queued_prompt_count(),
                    crate::QUEUE_DEPTH_MAX,
                    "parked={parked}"
                );

                assert_eq!(
                    view.enqueue_prompt(
                        "one too many".to_string(),
                        crate::view::PromptOrigin::InApp
                    ),
                    None,
                    "parked={parked}"
                );
                assert_eq!(
                    view.queued_prompt_count(),
                    crate::QUEUE_DEPTH_MAX,
                    "parked={parked}: a refusal must not grow the queue"
                );
                if parked {
                    return;
                }
                // Both submit funnels report it rather than pretending it landed.
                assert_eq!(
                    view.send_prompt_text("typed".to_string(), cx),
                    crate::view::PromptDispatch::QueueFull
                );
                assert_eq!(
                    view.send_prompt_text_for_telegram("from phone".to_string(), cx),
                    crate::view::PromptDispatch::QueueFull
                );
            })
            .unwrap();
    }
}

// The phone watch a Telegram-originated prompt arms.

/// Only a prompt that came from the phone arms the phone watch, and only once
/// it actually goes out: queuing alone arms nothing, and draining an in-app
/// prompt never does.
#[gpui::test]
fn a_drained_telegram_prompt_arms_the_watch_and_an_in_app_one_never_does(
    cx: &mut gpui::TestAppContext,
) {
    use crate::view::PromptOrigin;
    for (origin, arms) in [(PromptOrigin::Telegram, true), (PromptOrigin::InApp, false)] {
        let window = make_test_view(cx);
        window
            .update(cx, |view, _window, cx| {
                view.set_turn_in_flight();
                view.enqueue_prompt("queued".to_string(), origin)
                    .expect("room in the queue");
                assert!(
                    !view.is_phone_turn_waiting(),
                    "{origin:?}: queuing alone must not arm the watch"
                );

                view.set_turn_idle();
                view.drain_next_queued_prompt_for_test(cx);
                assert_eq!(view.is_phone_turn_waiting(), arms, "{origin:?} drained");
            })
            .unwrap();
    }
}

#[gpui::test]
fn telegram_prompt_does_not_replace_in_app_queue_edit(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            let id = view
                .enqueue_prompt("local draft".to_string(), crate::view::PromptOrigin::InApp)
                .expect("room in the queue");
            view.begin_edit(id, cx);

            let dispatch = view.send_prompt_text_for_telegram("from phone".to_string(), cx);

            assert_eq!(dispatch, crate::view::PromptDispatch::Queued);
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
fn external_in_app_prompt_keeps_the_composer_edit_and_origin(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            let id = view
                .enqueue_prompt("local draft".to_string(), crate::view::PromptOrigin::InApp)
                .expect("room in the queue");
            view.begin_edit(id, cx);

            assert_eq!(
                view.send_external_prompt_text("from host".to_string(), cx),
                crate::view::PromptDispatch::Queued
            );
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec!["local draft".to_string(), "from host".to_string()]
            );
            assert_eq!(
                view.queue.pending_prompts[1].origin,
                crate::view::PromptOrigin::InApp
            );
            assert_eq!(view.queue.editing_prompt, Some(id));

            assert_eq!(
                view.send_prompt_text("revised local".to_string(), cx),
                crate::view::PromptDispatch::Queued
            );
            assert_eq!(
                texts(&view.queue.pending_prompts),
                vec!["revised local".to_string(), "from host".to_string()]
            );
            assert_eq!(view.queue.editing_prompt, None);
        })
        .unwrap();
}

// Host notices, which go on the wire now or not at all.

/// A connected view with a live handle, idle; the probe counts wire prompts.
fn connected_view(
    cx: &mut gpui::TestAppContext,
) -> (
    gpui::WindowHandle<crate::view::AgentChatView>,
    daruda_acp::HandleProbe,
) {
    let window = make_test_view(cx);
    let (handle, probe) = daruda_acp::AcpSessionHandle::detached_for_test();
    window
        .update(cx, |view, _window, _cx| {
            view.status = crate::view::AgentSessionStatus::Connected;
            view.attach_handle(handle);
        })
        .unwrap();
    (window, probe)
}

#[gpui::test]
fn internal_notice_sends_when_connected_idle_and_empty(cx: &mut gpui::TestAppContext) {
    let (window, mut probe) = connected_view(cx);
    window
        .update(cx, |view, _window, cx| {
            assert_eq!(
                view.try_send_internal_prompt_text("/clear literal notice".into(), cx),
                crate::view::InternalDispatch::Sent
            );
            assert_eq!(probe.drain().0, 1);
            assert!(!view.turn_is_idle(), "the notice is the turn in flight");
            assert!(matches!(
                view.items.last(),
                Some(daruda_acp::ChatItem::UserText(text)) if text == "/clear literal notice"
            ));
            assert!(!view.is_phone_turn_waiting());
        })
        .unwrap();
}

#[gpui::test]
fn solicited_internal_notice_arms_completion_only_after_dispatch(cx: &mut gpui::TestAppContext) {
    let (window, mut probe) = connected_view(cx);
    window
        .update(cx, |view, _window, cx| {
            assert_eq!(
                view.try_send_internal_prompt_text_with_phone_mode(
                    "/clear literal notice".into(),
                    Some(crate::phone_turn::PhoneTurnMode::CompletionOnly),
                    cx,
                ),
                crate::view::InternalDispatch::Sent
            );
            assert_eq!(probe.drain().0, 1);
            assert!(view.phone_turn().is_some());
            assert!(!view.is_phone_turn_waiting());
            assert!(!view.take_phone_fallback_if_overdue(
                std::time::Instant::now() + std::time::Duration::from_secs(120),
                60
            ));
            view.items.push(daruda_acp::ChatItem::AssistantText {
                text: "final answer".into(),
                streaming: false,
                message_id: Some("answer".into()),
                phase: Default::default(),
            });
            assert!(view.take_phone_first_response().is_none());
            assert_eq!(view.finish_phone_turn(), crate::view::PhoneAckEffect::None);
            assert!(!view.phone_turn().unwrap().already_sent(Some("answer")));
        })
        .unwrap();
}

#[gpui::test]
fn refused_solicited_internal_notice_preserves_the_previous_phone_turn(
    cx: &mut gpui::TestAppContext,
) {
    for refusal in [
        crate::view::InternalDispatch::NotNow,
        crate::view::InternalDispatch::ReadOnly,
        crate::view::InternalDispatch::Unavailable,
    ] {
        let (window, probe) = connected_view(cx);
        if refusal == crate::view::InternalDispatch::Unavailable {
            drop(probe);
        }
        window
            .update(cx, |view, _window, cx| {
                view.start_phone_turn_for_test(std::time::Instant::now());
                let previous = view.phone_turn().cloned();
                match refusal {
                    crate::view::InternalDispatch::NotNow => view.set_turn_in_flight(),
                    crate::view::InternalDispatch::ReadOnly => {
                        view.set_access(daruda_store::tasks::AgentChatAccess::CliSnapshot(
                            daruda_store::tasks::ExecutionRef {
                                task_id: "task".into(),
                                execution_id: "execution".into(),
                            },
                        ))
                    }
                    _ => {}
                }
                assert_eq!(
                    view.try_send_internal_prompt_text_with_phone_mode(
                        "notice".into(),
                        Some(crate::phone_turn::PhoneTurnMode::CompletionOnly),
                        cx
                    ),
                    refusal
                );
                assert_eq!(view.phone_turn(), previous.as_ref());
            })
            .unwrap();
    }
}

#[gpui::test]
fn internal_notice_refuses_while_a_turn_is_in_flight(cx: &mut gpui::TestAppContext) {
    let (window, mut probe) = connected_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            assert_eq!(
                view.try_send_internal_prompt_text("notice".into(), cx),
                crate::view::InternalDispatch::NotNow
            );
            assert_eq!(view.queued_prompt_count(), 0, "a refusal never queues");
            assert_eq!(probe.drain().0, 0);
        })
        .unwrap();
}

/// Parked or live user prompts keep their place: the notice refuses rather
/// than queueing behind or jumping ahead, and the edit target survives.
#[gpui::test]
fn internal_notice_refuses_while_user_prompts_wait(cx: &mut gpui::TestAppContext) {
    for parked in [false, true] {
        let (window, mut probe) = connected_view(cx);
        window
            .update(cx, |view, _window, cx| {
                let queue = if parked {
                    &mut view.queue.paused_prompts
                } else {
                    &mut view.queue.pending_prompts
                };
                queue.push(queued(1, "user draft"));
                view.begin_edit(crate::view::PromptId(1), cx);
                assert_eq!(
                    view.try_send_internal_prompt_text("notice".into(), cx),
                    crate::view::InternalDispatch::NotNow,
                    "parked={parked}"
                );
                assert_eq!(view.queued_prompt_count(), 1, "parked={parked}");
                assert_eq!(
                    view.queue.editing_prompt,
                    Some(crate::view::PromptId(1)),
                    "parked={parked}"
                );
                assert_eq!(probe.drain().0, 0, "parked={parked}");
            })
            .unwrap();
    }
}

#[gpui::test]
fn internal_notice_refuses_during_stop_cancel_ack(cx: &mut gpui::TestAppContext) {
    let (window, mut probe) = connected_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            view.cancel_turn(cx);
            assert!(view.turn_awaiting_cancel_ack());
            probe.drain();
            assert_eq!(
                view.try_send_internal_prompt_text("notice".into(), cx),
                crate::view::InternalDispatch::NotNow
            );
            assert!(view.turn_awaiting_cancel_ack());
            assert_eq!(view.queued_prompt_count(), 0);
            assert_eq!(probe.drain().0, 0);
        })
        .unwrap();
}

#[gpui::test]
fn internal_notice_refuses_before_the_session_connects(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            assert_eq!(
                view.try_send_internal_prompt_text("notice".into(), cx),
                crate::view::InternalDispatch::NotNow
            );
            assert_eq!(view.queued_prompt_count(), 0);
            assert!(view.items.is_empty());
        })
        .unwrap();
}
