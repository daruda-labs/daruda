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
    assert_eq!(queued(cx).len(), usize::from(!connected));
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
    crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let wait = cx.update(|cx| {
        produce("covered", cx);
        RunsHost::open_run_wait(None, cx).unwrap().0
    });
    cx.run_until_parked();
    assert!(queued(cx).is_empty());
    cx.update(|cx| {
        crate::orchestrator::seed_control_surface_for_test(cx);
        assert!(crate::orchestrator::mcp_server(cx).is_some());
    });
    cx.run_until_parked();
    assert!(queued(cx).is_empty());
    cx.update(|cx| RunsHost::finish_run_wait(wait, false, cx));
    cx.run_until_parked();
    assert_eq!(queued(cx).len(), 1);
    cx.update(|cx| RunsHost::finish_run_wait(wait, false, cx));
    cx.run_until_parked();
    assert_eq!(queued(cx).len(), 1);
}
