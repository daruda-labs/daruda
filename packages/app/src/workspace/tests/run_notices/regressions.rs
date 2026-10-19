use super::*;

#[gpui::test]
fn notice_queue_preserves_a_quiesced_turn_completion(cx: &mut TestAppContext) {
    quiesced_completion(cx, false);
}

#[gpui::test]
fn notice_send_preserves_a_quiesced_turn_completion(cx: &mut TestAppContext) {
    quiesced_completion(cx, true);
}

fn quiesced_completion(cx: &mut TestAppContext, connected: bool) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let ws = host(cx);
    let answer = cx.update(|cx| crate::control::ask::wait_for(pane, cx).2);
    let (handle, mut probe) = daruda_acp::AcpSessionHandle::detached_for_test();
    ws.update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, _| {
                if connected {
                    view.set_status_for_test(daruda_chat::view::AgentSessionStatus::Connected);
                    view.attach_handle(handle);
                }
                view.set_turn_idle();
                view.owe_completion_for_test(daruda_chat::view::TurnOutcome::Completed);
                view.set_subagent_activity_for_test(
                    "finished-child",
                    std::time::Instant::now() - std::time::Duration::from_secs(20),
                );
            });
        produce("quiesced", cx);
    });
    cx.run_until_parked();
    assert!(
        queued(cx).is_empty(),
        "a notice is sent or kept, never parked"
    );
    assert_eq!(probe.drain().0, usize::from(connected));
    assert!(
        matches!(
            answer.try_recv(),
            Ok(Some(crate::control::result::PaneAnswer::NoAnswer))
        ),
        "the old turn must complete before the notice queues"
    );
    let (next_id, _, next_answer) = cx.update(|cx| crate::control::ask::wait_for(pane, cx));
    ws.update(cx, |ws, cx| {
        assert_eq!(
            ws.agent_chat_view(pane.pane)
                .unwrap()
                .read(cx)
                .last_reconciled_busy(),
            connected
        );
        ws.pulse_agent_chats(cx);
    });
    cx.run_until_parked();
    assert!(
        next_answer.try_recv().is_err(),
        "the old completion cannot settle a new waiter"
    );
    cx.update(|cx| {
        crate::control::ask::withdraw(pane, next_id, cx);
    });
}

#[gpui::test]
fn withdrawing_before_reservation_retries_on_the_replacement_gate(cx: &mut TestAppContext) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let _probe = connect(pane, cx);
    let wait = cx.update(|cx| {
        produce("covered", cx);
        RunsHost::open_run_wait(None, cx).unwrap().0
    });
    cx.run_until_parked();
    assert!(sent(cx).is_empty());
    cx.update(|cx| {
        crate::orchestrator::seed_control_surface_for_test(cx);
        assert!(crate::orchestrator::session_tools(cx).is_some());
    });
    cx.run_until_parked();
    assert!(sent(cx).is_empty());
    cx.update(|cx| RunsHost::finish_run_wait(wait, false, cx));
    cx.run_until_parked();
    assert_eq!(sent(cx).len(), 1);
    cx.update(|cx| RunsHost::finish_run_wait(wait, false, cx));
    cx.run_until_parked();
    assert_eq!(sent(cx).len(), 1);
}

#[gpui::test]
fn a_notice_waits_behind_queued_prompts_and_survives_their_discard(cx: &mut TestAppContext) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let _probe = connect(pane, cx);
    let ws = host(cx);
    ws.update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, _| view.fill_queue_for_test(1));
        produce("behind", cx);
    });
    cx.run_until_parked();
    assert_eq!(queued(cx).len(), 1, "only the person's prompt is queued");
    assert!(sent(cx).is_empty());
    ws.update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, cx| view.clear_queue(cx));
    });
    cx.run_until_parked();
    let prompts = sent(cx);
    assert_eq!(
        prompts.len(),
        1,
        "discarding the queue cannot lose the notice"
    );
    assert!(prompts[0].contains("flow:behind"));
}

#[gpui::test]
fn a_replaced_session_in_the_same_pane_hears_unreported_episodes_again(cx: &mut TestAppContext) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let _probe = connect(pane, cx);
    let ws = host(cx);
    let session = |id: &str, cx: &mut TestAppContext| {
        let id = id.to_owned();
        ws.update(cx, |ws, cx| {
            ws.agent_chat_view(pane.pane)
                .unwrap()
                .update(cx, |view, _| view.set_session_id_for_test(Some(id)));
        });
    };
    session("first", cx);
    cx.update(|cx| produce("carried", cx));
    cx.run_until_parked();
    assert_eq!(sent(cx).len(), 1);
    complete(pane, cx);
    session("second", cx);
    cx.update(|cx| {
        crate::orchestrator::seed_control_surface_for_test(cx);
        crate::orchestrator::session_tools(cx);
    });
    cx.run_until_parked();
    let prompts = sent(cx);
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("flow:carried"));
}

/// End the orchestrator's current turn as the agent does, so the chat pumps
/// its next queued prompt and the reconciled completion edge fires.
fn end_turn(pane: crate::telegram::bridge::PaneRef, cx: &mut TestAppContext) {
    host(cx).update(cx, |ws, cx| {
        let outcome = ws
            .agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, cx| {
                view.apply_event(
                    daruda_acp::AcpEvent::TurnEnded {
                        completed_normally: true,
                        stop_reason: "EndTurn".into(),
                        usage: None,
                    },
                    "",
                    false,
                    cx,
                );
                view.tick_activity(std::time::Instant::now(), cx)
            });
        if let Some(outcome) = outcome {
            ws.fire_activity_completion(pane.pane, outcome, cx);
        }
    });
    cx.run_until_parked();
}

#[gpui::test]
fn a_notice_is_sent_after_the_users_queued_prompts_drain(cx: &mut TestAppContext) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let _probe = connect(pane, cx);
    let ws = host(cx);
    ws.update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, cx| {
                view.set_turn_in_flight();
                view.send_prompt_text("first user".into(), cx);
                view.send_prompt_text("second user".into(), cx);
            });
        produce("drained", cx);
    });
    cx.run_until_parked();
    assert_eq!(queued(cx), vec!["first user", "second user"]);
    assert!(sent(cx).is_empty());
    end_turn(pane, cx);
    assert_eq!(sent(cx), vec!["first user"], "the user's prompt goes first");
    end_turn(pane, cx);
    assert_eq!(sent(cx), vec!["first user", "second user"]);
    end_turn(pane, cx);
    let prompts = sent(cx);
    assert_eq!(
        prompts.len(),
        3,
        "the drained queue lets the notice through"
    );
    assert!(prompts[2].contains("flow:drained"));
    assert!(queued(cx).is_empty(), "the notice never entered the queue");
}
