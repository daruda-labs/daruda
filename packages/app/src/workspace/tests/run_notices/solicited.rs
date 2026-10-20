use super::*;
use daruda_chat::view::{PhoneAckEffect, TurnOutcome};
use futures::{FutureExt as _, StreamExt as _};

#[gpui::test]
fn a_solicited_notice_reports_completion_once_while_present(cx: &mut TestAppContext) {
    notice_completion(true, false, cx);
}

#[gpui::test]
fn a_solicited_notice_reports_failure_once_while_present(cx: &mut TestAppContext) {
    notice_completion(true, true, cx);
}

#[gpui::test]
fn a_desktop_notice_keeps_each_channels_presence_rule(cx: &mut TestAppContext) {
    notice_completion(false, false, cx);
}

#[gpui::test]
async fn a_flow_captures_solicitation_before_reentering_its_workspace(cx: &mut TestAppContext) {
    use crate::control::{
        exec::{self, CommandOrigin},
        result::{ControlResult, LaneHandle},
        spec::{ResolvedCommand, ResolvedFlowCommand},
    };
    let fixture = crate::test_support::workspace_with_agent_chat(cx);
    let view = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.workspace.update(cx, |ws, cx| {
                let pane = ws
                    .seed_orchestrator_chat_pane_unrevealed_for_test(
                        daruda_config::AgentDefinition::claude_default().id,
                        fixture.root().to_owned(),
                        daruda_store::accounts::AccountSelection::SystemDefault,
                        None,
                        window,
                        cx,
                    )
                    .unwrap();
                ws.agent_chat_view(pane).unwrap().clone()
            })
        })
        .unwrap();
    cx.update(|cx| {
        WindowRegistry::register_orchestrator(
            fixture.window.into(),
            fixture.workspace.downgrade(),
            cx,
        )
    });
    view.update(cx, |view, _| {
        view.start_phone_turn_for_test(std::time::Instant::now())
    });
    let flows = crate::workspace::flow_paths::flows_dir(fixture.root());
    std::fs::create_dir_all(&flows).unwrap();
    std::fs::write(
        flows.join("solicited.yaml"),
        "version: 1\nnodes:\n  - id: check\n    kind: command\n    run: \"true\"\n",
    )
    .unwrap();
    cx.executor().allow_parking();
    let (key, waiter, wake) = cx.update(|cx| {
        let lane = fixture.workspace.read(cx).control_active_lane();
        let started = exec::run(
            ResolvedCommand::Flow(ResolvedFlowCommand::Run {
                name: "solicited".into(),
                lane: LaneHandle::new(fixture.workspace.read(cx).uuid(), lane),
            }),
            CommandOrigin::Orchestrator,
            cx,
        )
        .unwrap();
        let ControlResult::FlowStarting {
            handle: daruda_control_types::runs::RunHandle::Flow { run_id },
            ..
        } = started
        else {
            panic!("flow started")
        };
        let key = RunKey::Flow(run_id);
        let (waiter, wake) = RunsHost::open_run_wait(Some(vec![key.clone()]), cx).unwrap();
        (key, waiter, wake)
    });
    view.update(cx, |view, _| view.end_phone_turn());
    loop {
        if let Some(event) = cx.update(|cx| {
            RunsHost::unreported_run_events(cx)
                .into_iter()
                .find(|event| event.snapshot.key == key)
        }) {
            assert!(event.solicited);
            break;
        }
        wake.recv().await.unwrap();
    }
    cx.update(|cx| RunsHost::finish_run_wait(waiter, false, cx));
}

#[gpui::test]
async fn a_task_keeps_solicitation_after_its_approval_and_phone_turn_close(
    cx: &mut TestAppContext,
) {
    use crate::control::approval::{ApprovalChoice, resolve_only_pending_for_test};
    use crate::control::{
        exec::{self, CommandOrigin, Dispatch},
        result::ControlResult,
        spec::GatedCommand,
    };
    let fixture = crate::test_support::workspace_with_agent_chat(cx);
    let pane = crate::test_support::register_test_orchestrator(cx);
    let workspace = host(cx);
    let view = workspace.read_with(cx, |ws, _| ws.agent_chat_view(pane.pane).unwrap().clone());
    let _connections = cx.update(crate::workspace::Workspace::fake_agent_connections_for_test);
    let lane = fixture
        .workspace
        .read_with(cx, |ws, _| ws.control_active_lane());
    let created = fixture.workspace.update(cx, |ws, cx| {
        ws.control_task_create(lane.project, "Solicited", "work".into(), Some(lane), cx)
            .unwrap()
    });
    cx.run_until_parked();
    let task = created.await.unwrap();
    let _outbound = cx.update(crate::remote_channel::global::tests::setup);
    view.update(cx, |view, _| {
        view.start_phone_turn_for_test(std::time::Instant::now())
    });
    let dispatch = cx.update(|cx| {
        exec::run_gated(
            GatedCommand::TaskStart { task },
            CommandOrigin::Orchestrator,
            cx,
        )
    });
    let Dispatch::Deferred { outcome, .. } = dispatch else {
        panic!("approval required")
    };
    view.update(cx, |view, _| view.end_phone_turn());
    cx.update(|cx| resolve_only_pending_for_test(ApprovalChoice::Approved, cx));
    cx.run_until_parked();
    assert!(matches!(
        outcome.recv().await,
        Ok(Some(Ok(ControlResult::TaskStarted { .. })))
    ));
    cx.update(|cx| {
        let key = RunsHost::run_scope(cx).pop().unwrap();
        RunsHost::seed_run_wait(
            RunSnapshot {
                key,
                what: "solicited task".into(),
                why: None,
                state: RunState::Ended {
                    ended_as: EndedAs::Completed,
                    text: None,
                    pointers: Default::default(),
                },
            },
            cx,
        );
        assert!(RunsHost::unreported_run_events(cx)[0].solicited);
        assert!(!CommandOrigin::Orchestrator.is_solicited(cx));
    });
}

fn notice_completion(solicited: bool, failed: bool, cx: &mut TestAppContext) {
    let pane = crate::test_support::register_test_orchestrator(cx);
    install(cx);
    let mut probe = connect(pane, cx);
    let mut outbound = cx.update(|cx| {
        crate::app_presence::init(cx);
        crate::app_presence::seed_for_test(
            crate::platform::presence::AwaySignal::HERE,
            true,
            Some(std::time::Duration::ZERO),
            cx,
        );
        let outbound = crate::remote_channel::global::tests::setup(cx);
        let snapshot = ended("solicited-result");
        RunsHost::register_run_solicited(snapshot.key.clone(), solicited, cx);
        RunsHost::seed_run_wait(snapshot, cx);
        outbound
    });
    cx.run_until_parked();
    assert_eq!(probe.drain().0, 1);
    assert!(outbound.next().now_or_never().is_none(), "no received ACK");
    let workspace = host(cx);
    workspace.update(cx, |ws, cx| {
        ws.agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, cx| {
                assert_eq!(view.phone_turn().is_some(), solicited);
                assert!(!view.is_phone_turn_waiting());
                assert!(!view.take_phone_fallback_if_overdue(
                    std::time::Instant::now() + std::time::Duration::from_secs(120),
                    60,
                ));
                view.seed_items_for_test(
                    [daruda_acp::ChatItem::AssistantText {
                        text: "final result".into(),
                        streaming: false,
                        message_id: Some("result-message".into()),
                        phase: Default::default(),
                    }],
                    cx,
                );
                assert!(!view.take_phone_first_response_for_test());
            });
    });
    assert!(
        outbound.next().now_or_never().is_none(),
        "no first response or fallback"
    );
    workspace.update(cx, |ws, cx| {
        let outcome = ws
            .agent_chat_view(pane.pane)
            .unwrap()
            .update(cx, |view, cx| {
                let event = if failed {
                    daruda_acp::AcpEvent::TurnFailed(daruda_acp::AcpFailure::unclassified(
                        "notice failed",
                    ))
                } else {
                    daruda_acp::AcpEvent::TurnEnded {
                        completed_normally: true,
                        stop_reason: "EndTurn".into(),
                        usage: None,
                    }
                };
                assert_eq!(view.apply_event(event, "", false, cx), PhoneAckEffect::None);
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
    let mut channels = Vec::new();
    while let Some(Some(pending)) = outbound.next().now_or_never() {
        channels.push(pending.id);
        let crate::remote_channel::bridge::Outbound::Ping(ping) = pending.outbound else {
            panic!("a completion is a pane-attributed ping")
        };
        assert_eq!(ping.pane.pane, pane.pane);
        if !failed {
            assert!(
                matches!(ping.tail, crate::remote_channel::bridge::MessageTail::Markdown(text) if text.contains("final result"))
            );
        }
    }
    channels.sort();
    assert_eq!(
        channels,
        if solicited {
            vec!["personal", "work"]
        } else {
            vec!["personal"]
        }
    );
    workspace.update(cx, |ws, cx| {
        assert!(
            ws.agent_chat_view(pane.pane)
                .unwrap()
                .read(cx)
                .phone_turn()
                .is_none()
        );
        ws.pulse_agent_chats(cx);
    });
    cx.run_until_parked();
    assert!(
        outbound.next().now_or_never().is_none(),
        "completion is sent once"
    );
}
