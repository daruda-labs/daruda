use crate::control::approval::*;
use crate::remote_channel::global::Delivery;
use crate::surface::strings as s;
use daruda_chat::view::AgentChatView;
use gpui::{AppContext as _, TestAppContext};
use smol::channel::Receiver;
use std::time::Duration;

#[path = "orchestrator_approvals/announcements.rs"]
mod announcements;

fn setup(cx: &mut TestAppContext) -> Receiver<crate::telegram::bridge::Outbound> {
    setup_bridge(cx, true, Some(42))
}

fn setup_bridge(
    cx: &mut TestAppContext,
    enabled: bool,
    chat_id: Option<i64>,
) -> Receiver<crate::telegram::bridge::Outbound> {
    let (tx, rx) = smol::channel::unbounded();
    cx.update(|cx| {
        let mut config = crate::settings_store::SettingsStore::global(cx)
            .user()
            .clone();
        config.telegram.enabled = enabled;
        config.telegram.authorized_chat_id = chat_id;
        cx.global_mut::<crate::settings_store::SettingsStore>()
            .set_user_for_testing(config);
        let mut outbound = crate::telegram::global::install_for_test(enabled, chat_id, cx);
        cx.background_executor()
            .spawn(async move {
                use futures::StreamExt as _;
                while let Some(message) = outbound.next().await {
                    if tx.send(message).await.is_err() {
                        break;
                    }
                }
            })
            .detach();
    });
    rx
}

fn desktop_host(
    cx: &mut TestAppContext,
) -> (
    crate::test_support::ControlFixture,
    gpui::Entity<AgentChatView>,
) {
    let fixture = crate::test_support::workspace_with_agent_chat(cx);
    let view = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.workspace.update(cx, |ws, cx| {
                let agent = crate::settings_store::SettingsStore::global(cx)
                    .user()
                    .resolved_agents()[0]
                    .id
                    .clone();
                let pane = ws
                    .seed_orchestrator_chat_pane_unrevealed_for_test(
                        agent,
                        std::env::temp_dir(),
                        daruda_store::accounts::AccountSelection::SystemDefault,
                        None,
                        window,
                        cx,
                    )
                    .unwrap();
                let view = ws.agent_chat_view(pane).unwrap().clone();
                view.update(cx, |view, _| {
                    view.set_status_for_test(daruda_chat::view::AgentSessionStatus::Connected);
                });
                view
            })
        })
        .unwrap();
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register_orchestrator(
            fixture.window.into(),
            fixture.workspace.downgrade(),
            cx,
        );
    });
    (fixture, view)
}

#[gpui::test]
fn desktop_without_a_phone_can_answer_a_hidden_orchestrator_card(cx: &mut TestAppContext) {
    let (fixture, view) = desktop_host(cx);
    let initial_focus = fixture
        .workspace
        .read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
    let outbound = setup_bridge(cx, false, None);
    let (id, answer) = cx.update(|cx| {
        request(
            "Create a task".into(),
            Delivery::Presence { away: false },
            cx,
        )
    });
    cx.run_until_parked();
    assert!(answer.is_empty());
    assert!(outbound.is_empty());
    assert_eq!(
        view.read_with(cx, |view, _| view.host_cards_for_test()),
        vec![(id.0, None)]
    );
    fixture.workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().focused_pane_id, initial_focus);
    });

    view.update(cx, |_, cx| {
        cx.emit(daruda_chat::view::AgentChatEvent::HostCardChosen {
            card_id: id.0,
            action_id: "approve".into(),
        });
    });
    cx.run_until_parked();
    assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Approved));
    assert_eq!(
        view.read_with(cx, |view, _| view.host_cards_for_test()),
        vec![(id.0, Some(s::control::approval_approved_desktop()))]
    );
    assert!(!cx.update(|cx| resolve(id, ApprovalChoice::Refused, Answerer::Telegram, cx)));
}

#[gpui::test]
fn a_failed_phone_queue_leaves_the_desktop_request_answerable(cx: &mut TestAppContext) {
    let (_fixture, view) = desktop_host(cx);
    let outbound = cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    drop(outbound);
    let (id, answer) = cx.update(|cx| request("Create a task".into(), Delivery::Explicit, cx));
    cx.run_until_parked();
    assert!(answer.is_empty());
    assert_eq!(
        view.read_with(cx, |view, _| view.host_cards_for_test()),
        vec![(id.0, None)]
    );
    cx.update(|cx| {
        assert_eq!(
            crate::telegram::global::pending_approval_tokens_for_test(cx),
            0
        );
        assert!(resolve_desktop(id.0, "refuse", cx));
    });
    assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Refused));
}

#[gpui::test]
fn explicit_orchestrator_stop_withdraws_after_the_foreground_turn_already_completed(
    cx: &mut TestAppContext,
) {
    let (fixture, view) = desktop_host(cx);
    let _outbound = setup_bridge(cx, false, None);
    let pane = fixture
        .workspace
        .read_with(cx, |ws, _| ws.orchestrator_chat_pane().unwrap().pane);
    for escape in [false, true] {
        view.update(cx, |view, cx| {
            view.set_turn_idle();
            view.owe_completion_for_test(daruda_chat::view::TurnOutcome::Completed);
            view.seed_items_for_test(
                [daruda_acp::ChatItem::ToolCall(daruda_acp::ToolCallItem {
                    id: "child".into(),
                    title: "Background task".into(),
                    kind: daruda_acp::ToolKindView::Read,
                    tool_name: None,
                    status: daruda_acp::ToolStatusView::InProgress,
                    diffs: Vec::new(),
                    output: Vec::new(),
                    raw_input: None,
                    locations: Vec::new(),
                    parent_tool_id: Some("parent".into()),
                    exit: None,
                })],
                cx,
            );
            assert!(view.is_busy());
        });
        let (id, answer) = cx.update(|cx| {
            request(
                "Create a task".into(),
                Delivery::Presence { away: false },
                cx,
            )
        });
        cx.run_until_parked();
        fixture.workspace.update(cx, |ws, cx| {
            if escape {
                assert!(ws.cancel_agent_turn_if_active(pane, cx));
            } else {
                ws.cancel_agent_turn(pane, cx);
            }
        });
        cx.run_until_parked();
        assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Withdrawn));
        assert_eq!(
            view.read_with(cx, |view, _| view.host_cards_for_test()),
            vec![(id.0, Some(s::control::approval_cancelled()))]
        );
    }
    let (id, answer) = cx.update(|cx| {
        request(
            "Create a task".into(),
            Delivery::Presence { away: false },
            cx,
        )
    });
    fixture
        .workspace
        .update(cx, |ws, cx| ws.cancel_agent_turn(pane, cx));
    cx.run_until_parked();
    assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Withdrawn));
    assert!(!cx.update(|cx| is_waiting(id, cx)));
}

#[gpui::test]
fn withdrawal_defers_card_updates_until_workspace_and_view_borrows_end(cx: &mut TestAppContext) {
    let (fixture, view) = desktop_host(cx);
    let _outbound = setup_bridge(cx, false, None);
    let (id, answer) = cx.update(|cx| {
        request(
            "Create a task".into(),
            Delivery::Presence { away: false },
            cx,
        )
    });
    cx.run_until_parked();
    fixture.workspace.update(cx, |_, cx| {
        view.update(cx, |_, cx| assert!(withdraw(id, cx)));
    });
    cx.run_until_parked();
    assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Withdrawn));
    assert_eq!(
        view.read_with(cx, |view, _| view.host_cards_for_test()),
        vec![(id.0, Some(s::control::approval_cancelled()))]
    );
}

#[gpui::test]
fn a_remote_decision_and_timeout_replace_desktop_choices(cx: &mut TestAppContext) {
    let (_fixture, view) = desktop_host(cx);
    let _outbound = setup(cx);
    for choice in [ApprovalChoice::Approved, ApprovalChoice::Refused] {
        let (id, answer) = cx.update(|cx| request("Create a task".into(), Delivery::Explicit, cx));
        cx.run_until_parked();
        assert!(cx.update(|cx| resolve(id, choice, Answerer::Telegram, cx)));
        cx.run_until_parked();
        assert_eq!(answer.try_recv(), Ok(choice.into()));
        let label = match choice {
            ApprovalChoice::Approved => s::control::approval_approved_telegram(),
            ApprovalChoice::Refused => s::control::approval_refused_telegram(),
        };
        assert_eq!(
            view.read_with(cx, |view, _| view.host_cards_for_test()),
            vec![(id.0, Some(label))]
        );
    }
    let (id, answer) = cx.update(|cx| request("Create a task".into(), Delivery::Explicit, cx));
    cx.run_until_parked();
    cx.executor()
        .advance_clock(APPROVAL_TIMEOUT + Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::TimedOut));
    assert_eq!(
        view.read_with(cx, |view, _| view.host_cards_for_test()),
        vec![(id.0, Some(s::control::approval_timed_out()))]
    );
}

#[gpui::test]
fn local_approvals_follow_each_telegram_connections_presence_setting(cx: &mut TestAppContext) {
    let (_fixture, view) = desktop_host(cx);
    let outbound = setup(cx);
    for (only_when_away, delivery, expected) in [
        (true, Delivery::Presence { away: false }, false),
        (false, Delivery::Presence { away: false }, true),
        (true, Delivery::Presence { away: true }, true),
        (true, Delivery::Explicit, true),
    ] {
        cx.update(|cx| {
            let mut config = crate::settings_store::SettingsStore::global(cx)
                .user()
                .clone();
            config.telegram.only_when_away = only_when_away;
            cx.global_mut::<crate::settings_store::SettingsStore>()
                .set_user_for_testing(config);
        });
        let (id, answer) = cx.update(|cx| request("Create a task".into(), delivery, cx));
        cx.run_until_parked();
        assert!(answer.is_empty());
        assert_eq!(outbound.try_recv().is_ok(), expected);
        assert!(view.read_with(cx, |view, _| {
            view.host_cards_for_test().contains(&(id.0, None))
        }));
        cx.update(|cx| assert!(withdraw(id, cx)));
    }
}

#[gpui::test]
fn gated_requests_capture_desktop_or_phone_turn_delivery_at_creation(cx: &mut TestAppContext) {
    use crate::control::exec::{CommandOrigin, Dispatch, Pending};
    let (fixture, view) = desktop_host(cx);
    let outbound = setup(cx);
    let lane = fixture.workspace.read_with(cx, |ws, _| {
        crate::control::result::LaneHandle::new(ws.uuid(), ws.control_active_lane())
    });
    for (origin, phone_turn, expected) in [
        (CommandOrigin::Orchestrator, false, false),
        (CommandOrigin::Orchestrator, true, true),
        (CommandOrigin::Phone, false, true),
    ] {
        view.update(cx, |view, _| {
            if phone_turn {
                view.start_phone_turn_for_test(std::time::Instant::now());
            } else {
                view.end_phone_turn();
            }
        });
        let dispatch = cx.update(|cx| {
            crate::control::exec::run_gated(
                crate::control::spec::GatedCommand::ChatNew { lane, agent: None },
                origin,
                cx,
            )
        });
        let Dispatch::Deferred {
            pending: Pending::Approval(id),
            ..
        } = dispatch
        else {
            panic!("a valid command must wait for approval");
        };
        view.update(cx, |view, _| view.end_phone_turn());
        cx.run_until_parked();
        assert_eq!(outbound.try_recv().is_ok(), expected);
        assert!(cx.update(|cx| is_waiting(id, cx)));
        cx.update(|cx| assert!(withdraw(id, cx)));
    }
}

#[gpui::test]
fn pending_approvals_notify_every_windows_chip_without_reading_the_host(cx: &mut TestAppContext) {
    use crate::workspace::status_bar::orchestrator_chip::OrchestratorChipState;
    use std::cell::Cell;
    use std::rc::Rc;

    let (host, _view) = desktop_host(cx);
    let other = crate::test_support::workspace_with_agent_chat(cx);
    let _outbound = setup_bridge(cx, false, None);
    // A hosted session implies the feature is on; with it off, no window
    // shows a chip at all.
    cx.update(|cx| {
        let mut config = crate::settings_store::SettingsStore::global(cx)
            .user()
            .clone();
        config.orchestrator.enabled = true;
        cx.global_mut::<crate::settings_store::SettingsStore>()
            .set_user_for_testing(config);
    });
    let notifications = [Rc::new(Cell::new(0)), Rc::new(Cell::new(0))];
    cx.update(|cx| {
        for (workspace, notifications) in [&host.workspace, &other.workspace]
            .into_iter()
            .zip(notifications.iter().cloned())
        {
            cx.observe(workspace, move |_, _| {
                notifications.set(notifications.get() + 1)
            })
            .detach();
        }
    });
    let (id, answer) = cx.update(|cx| {
        request(
            "Create a task".into(),
            Delivery::Presence { away: false },
            cx,
        )
    });
    cx.run_until_parked();
    host.workspace.update(cx, |ws, cx| {
        assert_eq!(
            ws.orchestrator_chip_state(cx),
            Some(OrchestratorChipState::AwaitingPermission)
        );
        other.workspace.update(cx, |ws, cx| {
            assert_eq!(
                ws.orchestrator_chip_state(cx),
                Some(OrchestratorChipState::AwaitingPermission)
            );
        });
    });
    assert!(notifications.iter().all(|count| count.get() > 0));
    let before = notifications.each_ref().map(|count| count.get());
    host.workspace.update(cx, |_, cx| {
        assert!(resolve_desktop(id.0, "refuse", cx));
    });
    cx.run_until_parked();
    assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Refused));
    for (count, before) in notifications.iter().zip(before) {
        assert!(count.get() > before);
    }
    other.workspace.update(cx, |ws, cx| {
        assert_ne!(
            ws.orchestrator_chip_state(cx),
            Some(OrchestratorChipState::AwaitingPermission)
        );
    });
}

#[gpui::test]
fn approval_notifications_honor_waiting_and_visible_pane_preferences(cx: &mut TestAppContext) {
    let (fixture, _view) = desktop_host(cx);
    let _outbound = setup_bridge(cx, false, None);
    for (enabled, skip_visible, visible) in [
        (false, false, false),
        (true, false, false),
        (true, true, false),
        (true, false, true),
        (true, true, true),
    ] {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.workspace.update(cx, |ws, cx| {
                ws.mirrors.notifications.agent_waiting_enabled = enabled;
                ws.mirrors.notifications.skip_focused_pane = skip_visible;
                if visible {
                    assert!(ws.show_orchestrator_tab(window, cx));
                } else {
                    ws.hide_orchestrator_tab(window, cx);
                }
            });
        })
        .unwrap();
        let focus = fixture
            .workspace
            .read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
        let _ = crate::platform::notifications::take_recorded_for_test();
        let expected =
            enabled && !(skip_visible && visible && crate::platform::attention::is_app_active());
        let (id, answer) = cx.update(|cx| {
            request(
                "Create a task".into(),
                Delivery::Presence { away: false },
                cx,
            )
        });
        cx.run_until_parked();
        let shown = crate::platform::notifications::take_recorded_for_test();
        assert_eq!(shown.len(), usize::from(expected));
        if expected {
            assert_eq!(shown[0].1, s::notification::agent_waiting());
        }
        assert!(answer.is_empty());
        fixture.workspace.read_with(cx, |ws, _| {
            assert_eq!(ws.active_runtime().focused_pane_id, focus);
        });
        cx.update(|cx| assert!(withdraw(id, cx)));
    }
}
