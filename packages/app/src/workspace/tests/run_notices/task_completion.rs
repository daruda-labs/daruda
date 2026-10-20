use super::*;
use crate::control::exec::{self, CommandOrigin};
use daruda_chat::view::TurnOutcome;
use futures::{FutureExt as _, StreamExt as _};

#[derive(Clone, Copy)]
enum Conversation {
    Orchestrated,
    ExplicitChild,
    DirectPhone,
    ManualContinuation,
}

#[gpui::test]
async fn fast_task_success_away_reports_only_the_orchestrator_answer(cx: &mut TestAppContext) {
    task_completion(Conversation::Orchestrated, false, true, cx).await;
}

#[gpui::test]
async fn fast_task_error_away_reports_only_the_orchestrator_answer(cx: &mut TestAppContext) {
    task_completion(Conversation::Orchestrated, true, true, cx).await;
}

#[gpui::test]
async fn fast_task_success_present_reports_only_the_orchestrator_answer(cx: &mut TestAppContext) {
    task_completion(Conversation::Orchestrated, false, false, cx).await;
}

#[gpui::test]
async fn fast_task_error_present_reports_only_the_orchestrator_answer(cx: &mut TestAppContext) {
    task_completion(Conversation::Orchestrated, true, false, cx).await;
}

#[gpui::test]
async fn an_explicit_child_phone_turn_keeps_its_completion(cx: &mut TestAppContext) {
    task_completion(Conversation::ExplicitChild, false, false, cx).await;
}

#[gpui::test]
async fn an_explicit_child_phone_turn_keeps_its_failure(cx: &mut TestAppContext) {
    task_completion(Conversation::ExplicitChild, true, false, cx).await;
}

#[gpui::test]
async fn a_direct_phone_task_keeps_its_presence_completion(cx: &mut TestAppContext) {
    task_completion(Conversation::DirectPhone, false, true, cx).await;
}

#[gpui::test]
async fn a_direct_phone_task_present_keeps_each_channels_presence_rule(cx: &mut TestAppContext) {
    task_completion(Conversation::DirectPhone, false, false, cx).await;
}

#[gpui::test]
async fn a_manual_continuation_after_task_settlement_keeps_its_completion(cx: &mut TestAppContext) {
    task_completion(Conversation::ManualContinuation, false, true, cx).await;
}

fn finish_chat(
    workspace: &gpui::Entity<crate::workspace::Workspace>,
    pane: crate::telegram::bridge::PaneRef,
    failed: bool,
    text: &str,
    cx: &mut TestAppContext,
) {
    workspace.update(cx, |ws, cx| {
        let outcome = ws
            .agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, cx| {
                view.seed_items_for_test(
                    [daruda_acp::ChatItem::AssistantText {
                        text: text.into(),
                        streaming: false,
                        message_id: Some(text.into()),
                        phase: Default::default(),
                    }],
                    cx,
                );
                let event = if failed {
                    daruda_acp::AcpEvent::TurnFailed(daruda_acp::AcpFailure::unclassified(
                        "child failed",
                    ))
                } else {
                    daruda_acp::AcpEvent::TurnEnded {
                        completed_normally: true,
                        stop_reason: "EndTurn".into(),
                        usage: None,
                    }
                };
                view.apply_event(event, "", false, cx);
                view.tick_activity(std::time::Instant::now(), cx)
            });
        assert_eq!(
            outcome,
            Some(if failed {
                TurnOutcome::Errored
            } else {
                TurnOutcome::Completed
            })
        );
        ws.fire_activity_completion(pane.pane, outcome.unwrap(), cx);
    });
    cx.run_until_parked();
}

async fn task_completion(
    conversation: Conversation,
    failed: bool,
    away: bool,
    cx: &mut TestAppContext,
) {
    let fixture = crate::test_support::workspace_with_agent_chat(cx);
    let orchestrator = crate::test_support::register_test_orchestrator(cx);
    let orchestrator_workspace = host(cx);
    install(cx);
    let mut probe = connect(orchestrator, cx);
    let _connections = cx.update(crate::workspace::Workspace::fake_agent_connections_for_test);
    let mut outbound = cx.update(|cx| {
        let outbound = crate::remote_channel::global::tests::setup(cx);
        crate::app_presence::seed_for_test(
            crate::platform::presence::AwaySignal::HERE,
            true,
            Some(std::time::Duration::from_secs(if away { 3600 } else { 0 })),
            cx,
        );
        outbound
    });
    let lane = fixture
        .workspace
        .read_with(cx, |ws, _| ws.control_active_lane());
    let created = fixture.workspace.update(cx, |ws, cx| {
        ws.control_task_create(lane.project, "Fast task", "work".into(), Some(lane), cx)
            .unwrap()
    });
    cx.run_until_parked();
    let task = created.await.unwrap();
    let origin = if matches!(conversation, Conversation::DirectPhone) {
        CommandOrigin::Phone
    } else {
        CommandOrigin::Orchestrator
    };
    let pending = cx.update(|cx| exec::begin_task_start(task.clone(), origin, true, cx).unwrap());
    cx.run_until_parked();
    let (execution, child) = fixture.workspace.read_with(cx, |ws, cx| {
        let task = cx.global::<crate::runs::GlobalTasks>().get(&task).unwrap();
        assert_eq!(task.started_by, Some(origin.task_started_by()));
        let execution = task.current_exec().unwrap().clone();
        assert!(RunsHost::run_scope(cx).is_empty());
        let pane = ws
            .every_agent_chat()
            .find_map(|(pane, _)| {
                (ws.task_chat_execution(pane, cx) == Some(execution.clone())).then_some(pane)
            })
            .unwrap();
        (
            execution,
            crate::telegram::bridge::PaneRef {
                workspace: ws.uuid(),
                pane,
            },
        )
    });
    if matches!(conversation, Conversation::ExplicitChild) {
        fixture.workspace.update(cx, |ws, cx| {
            ws.agent_chat_view(child.pane)
                .unwrap()
                .update(cx, |view, _| {
                    let started = std::time::Instant::now();
                    view.start_phone_turn_for_test(started);
                    assert!(view.take_phone_fallback_if_overdue(
                        started + std::time::Duration::from_secs(120),
                        60,
                    ));
                });
        });
    }
    finish_chat(&fixture.workspace, child, failed, "child result", cx);
    let expected_child_channels = match conversation {
        Conversation::ExplicitChild => vec!["personal", "work"],
        Conversation::DirectPhone if away => vec!["personal", "work"],
        Conversation::DirectPhone => vec!["personal"],
        Conversation::Orchestrated | Conversation::ManualContinuation => Vec::new(),
    };
    let mut child_channels = Vec::new();
    while let Some(Some(pending)) = outbound.next().now_or_never() {
        let crate::remote_channel::bridge::Outbound::Ping(ping) = pending.outbound else {
            panic!("a completion is a pane ping")
        };
        assert_eq!(ping.pane, child);
        child_channels.push(pending.id);
    }
    child_channels.sort();
    assert_eq!(child_channels, expected_child_channels);
    cx.update(|cx| {
        let task = cx.global::<crate::runs::GlobalTasks>().get(&task).unwrap();
        assert!(task.lifecycle.is_terminal());
        assert_eq!(task.lifecycle.exec(), Some(&execution));
    });
    assert!(cx.update(|cx| pending.try_finish(cx)).unwrap().is_ok());
    cx.run_until_parked();
    while outbound.next().now_or_never().flatten().is_some() {}
    if matches!(conversation, Conversation::DirectPhone) {
        assert_eq!(probe.drain().0, 0);
        return;
    }
    assert_eq!(
        probe.drain().0,
        1,
        "registration replays the fast cached end"
    );
    assert_eq!(sent(cx).len(), 1);
    assert!(sent(cx)[0].contains(&format!("task:{}/{}", task, execution.execution_id)));
    if matches!(conversation, Conversation::ManualContinuation) {
        fixture.workspace.update(cx, |ws, cx| {
            assert_eq!(
                RunsHost::pane_execution(ws.run_pane(child.pane), cx),
                Some(&execution)
            );
            ws.control_say(
                child.pane,
                "continue manually".into(),
                CommandOrigin::Orchestrator,
                cx,
            )
            .unwrap();
        });
        finish_chat(&fixture.workspace, child, false, "manual result", cx);
        let mut channels = Vec::new();
        while let Some(Some(pending)) = outbound.next().now_or_never() {
            let crate::remote_channel::bridge::Outbound::Ping(ping) = pending.outbound else {
                panic!("a manual completion is a pane ping")
            };
            assert_eq!(ping.pane, child);
            channels.push(pending.id);
        }
        channels.sort();
        assert_eq!(channels, vec!["personal", "work"]);
    }
    finish_chat(
        &orchestrator_workspace,
        orchestrator,
        failed,
        "orchestrator answer",
        cx,
    );
    let mut channels = Vec::new();
    while let Some(Some(pending)) = outbound.next().now_or_never() {
        let crate::remote_channel::bridge::Outbound::Ping(ping) = pending.outbound else {
            panic!("the final answer is a pane ping")
        };
        assert_eq!(ping.pane, orchestrator);
        channels.push(pending.id);
    }
    channels.sort();
    assert_eq!(channels, vec!["personal", "work"]);
    orchestrator_workspace.update(cx, |ws, cx| ws.pulse_agent_chats(cx));
    cx.run_until_parked();
    assert!(outbound.next().now_or_never().is_none());
}
