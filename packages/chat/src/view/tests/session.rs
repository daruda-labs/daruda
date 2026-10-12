//! One session's lifecycle: the replay gate, resume, failures and a fresh
//! session, the mode it remembers, and the permission requests it raises.

use super::*;

/// A `session/load` replay coalesces row rebuilding until it reaches a
/// terminal resume signal. The same gate must also release on failure paths so
/// replayed content is not left invisible.
#[gpui::test]
fn replay_gate_releases_on_connected_error_or_abort(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{AcpEvent, ChatItem};

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            // Simulate a resume mid-replay: gate open on the id the load asked
            // for, items populated by the replayed updates, rows not yet
            // projected.
            view.replay = crate::view::Replay::Loading {
                requested: "sess-1".into(),
            };
            view.items = vec![ChatItem::UserText("q".into()), assistant_text_item("a")];
            view.rows.clear();

            // A non-terminal event during the replay must NOT rebuild rows.
            view.apply_event(AcpEvent::Notice("still loading".into()), "", false, cx);
            assert!(view.is_replaying(), "gate stays set until Connected/Error");
            assert!(
                view.rows.is_empty(),
                "row rebuild is coalesced while replaying"
            );
            assert_eq!(view.items.len(), 2, "items still accumulate during replay");

            // Connected clears the gate and runs the catch-up rebuild.
            view.apply_event(
                AcpEvent::Connected {
                    program: None,
                    session_id: "sess-1".into(),
                    modes: None,
                    config_options: Vec::new(),
                    capabilities: Default::default(),
                    login_methods: Vec::new(),
                },
                "",
                false,
                cx,
            );
            assert!(!view.is_replaying(), "Connected releases the gate");
            assert_eq!(view.session_id.as_deref(), Some("sess-1"));
            assert!(
                !view.rows.is_empty(),
                "the catch-up rebuild projects the replayed items"
            );
            assert_eq!(
                view.items.len(),
                2,
                "resume keeps the replayed conversation"
            );

            // The end-of-stream guard releases a gate stuck with no terminal
            // event, projecting whatever accumulated.
            view.replay = crate::view::Replay::Loading {
                requested: "sess-1".into(),
            };
            view.items = vec![ChatItem::UserText("q".into())];
            view.rows.clear();
            view.abort_restore(cx);
            assert!(!view.is_replaying(), "abort_restore releases the gate");
            assert!(
                !view.rows.is_empty(),
                "abort_restore projects the accumulated items"
            );
        })
        .unwrap();
}

/// A `session/load` the agent cannot honour is silently downgraded to a fresh
/// `session/new`, which answers with a *different* id. The gate carries the id
/// it asked for, so that downgrade is detectable: the pane must run
/// fresh-session setup instead of keeping the prior session's title.
#[gpui::test]
fn a_downgraded_resume_is_not_mistaken_for_a_real_one(cx: &mut gpui::TestAppContext) {
    use daruda_acp::AcpEvent;

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

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            // Arm the gate the way a connect does — through the seam, so this
            // also pins that `begin_connect` opens it on the id it was handed.
            view.begin_connect(Some("sess-old".into()), cx);
            assert!(view.is_replaying(), "a resume target opens the gate");
            view.session_title = Some("prior conversation".into());
            // The agent answered the load with an id we never asked for.
            view.apply_event(connected("sess-new"), "", false, cx);
            assert_eq!(view.session_id.as_deref(), Some("sess-new"));
            assert!(
                view.session_title.is_none(),
                "a downgraded resume runs fresh-session setup"
            );

            // The same reply against the id the load asked for is a real resume.
            view.begin_connect(Some("sess-old".into()), cx);
            view.session_title = Some("prior conversation".into());
            view.apply_event(connected("sess-old"), "", false, cx);
            assert_eq!(
                view.session_title.as_deref(),
                Some("prior conversation"),
                "an honoured resume keeps the replayed session title"
            );

            // A fresh connect names no resume target, so it opens no gate.
            view.begin_connect(None, cx);
            assert!(
                !view.is_replaying(),
                "a fresh session/new has nothing to replay"
            );
        })
        .unwrap();
}

/// A `session/prompt` JSON-RPC failure keeps the ACP session alive: it shows an
/// inline error, settles the foreground turn, and records an errored completion
/// without switching the pane to terminal `Error`.
#[gpui::test]
fn turn_failed_keeps_session_connected_and_shows_error(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{AcpEvent, ChatItem};

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.status = crate::view::AgentSessionStatus::Connected;
            view.set_turn_in_flight();

            view.apply_event(
                AcpEvent::TurnFailed(daruda_acp::AcpFailure::unclassified(
                    "session limit reached",
                )),
                "",
                false,
                cx,
            );

            assert!(
                matches!(view.status, crate::view::AgentSessionStatus::Connected),
                "a per-turn failure must not kill the session, got {:?}",
                view.status
            );
            assert!(view.turn_is_idle(), "the failed turn must settle to idle");
            assert_eq!(
                view.items.last(),
                Some(&ChatItem::Failure(daruda_acp::AcpFailure::unclassified(
                    "session limit reached"
                ))),
                "the failure is surfaced as an inline Error item"
            );
            assert_eq!(
                view.activity.pending_completion,
                Some(crate::view::TurnOutcome::Errored)
            );
        })
        .unwrap();
}

#[gpui::test]
fn transport_eof_localizes_and_preserves_the_session_for_reconnect(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.session_id = Some("saved-session".into());
            view.status = crate::view::AgentSessionStatus::Connected;
            assert!(view.needs_disconnect_error());
            view.set_turn_in_flight();
            view.apply_event(
                daruda_acp::AcpEvent::Error(daruda_acp::AcpFailure::TransportClosed {
                    message: "Incoming transport closed".into(),
                }),
                "",
                false,
                cx,
            );
            let crate::view::AgentSessionStatus::Error { message, remedy } = &view.status else {
                panic!("EOF must terminate the session");
            };
            assert_eq!(*remedy, daruda_acp::Remedy::Retry);
            assert_eq!(message, &crate::strings::agent_chat::transport_closed());
            assert_ne!(message, "Incoming transport closed");
            assert_eq!(view.session_id.as_deref(), Some("saved-session"));
            assert!(view.turn_is_idle());
            assert!(view.any_handle().is_none());
            assert!(
                !view.needs_disconnect_error(),
                "do not emit a duplicate terminal failure"
            );
        })
        .unwrap();
}

#[gpui::test]
fn preparation_failure_releases_replay_without_losing_the_resume_target(
    cx: &mut gpui::TestAppContext,
) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.session_id = Some("saved-session".into());
            view.begin_connect(Some("saved-session".into()), cx);
            assert!(view.is_replaying());
            view.apply_event(
                daruda_acp::AcpEvent::Error(daruda_acp::AcpFailure::AdapterInstall {
                    kind: daruda_acp::preparation::PreparationKind::Network,
                    message: "npm ENOTFOUND".into(),
                }),
                "",
                false,
                cx,
            );
            assert!(!view.is_replaying());
            assert!(view.turn_is_idle());
            assert!(view.any_handle().is_none());
            assert_eq!(view.session_id.as_deref(), Some("saved-session"));
            let crate::view::AgentSessionStatus::Error { message, remedy } = &view.status else {
                panic!("preparation failure must terminate the attempt");
            };
            assert_eq!(*remedy, daruda_acp::Remedy::Retry);
            assert_eq!(
                message,
                &crate::strings::agent_chat::adapter_install_failed()
            );
        })
        .unwrap();
}

#[gpui::test]
fn invalid_package_has_localized_guidance_without_a_misleading_retry(
    cx: &mut gpui::TestAppContext,
) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.apply_event(
                daruda_acp::AcpEvent::Error(daruda_acp::AcpFailure::AdapterInstall {
                    kind: daruda_acp::preparation::PreparationKind::InvalidPackage,
                    message: "unsupported bin entry".into(),
                }),
                "",
                false,
                cx,
            );
            let crate::view::AgentSessionStatus::Error { message, remedy } = &view.status else {
                panic!("invalid package must surface a failure");
            };
            assert_ne!(*remedy, daruda_acp::Remedy::Retry);
            assert_eq!(
                message,
                &crate::strings::agent_chat::adapter_setup_required()
            );
        })
        .unwrap();
}

/// `/clear` resets the local session model before the workspace starts the
/// fresh connection.
#[gpui::test]
fn reset_for_new_session_clears_conversation_state(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{ChatItem, PlanEntryView, PlanPriority, PlanStatus};

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.session_id = Some("abc".to_string());
            view.items.push(ChatItem::UserText("hi".into()));
            view.items.push(ChatItem::UserText("again".into()));
            view.set_turn_in_flight();
            view.plan.push(PlanEntryView {
                content: "step 1".into(),
                priority: PlanPriority::Medium,
                status: PlanStatus::Pending,
            });
            view.fold
                .toggle(FoldKey::Thinking(1), FoldContext::past(true));
            assert!(
                !view
                    .fold
                    .is_expanded(&FoldKey::Thinking(1), FoldContext::past(true)),
                "sanity: override collapsed the block while active"
            );
            view.fold.set_mode(FoldPreset::Expanded.mode());
            view.activity.turn_records.insert(
                1,
                crate::view::TurnRecord {
                    worked_for: std::time::Duration::from_secs(9),
                    finished_at: chrono::Local::now(),
                    output_tokens: Some(42),
                },
            );

            view.reset_for_new_session(cx);

            assert!(view.items.is_empty(), "reset clears the conversation items");
            assert!(
                view.activity.turn_records.is_empty(),
                "turn records are keyed by item index with no session identity, \
                 so one left behind would label the next conversation's rows"
            );

            assert!(
                view.rows.is_empty(),
                "reset splices the projected rows to 0"
            );
            assert_eq!(
                view.session_id, None,
                "reset clears the persisted session id"
            );
            assert_eq!(
                view.status,
                crate::view::AgentSessionStatus::Connecting,
                "reset parks the view in Connecting for the fresh session/new"
            );
            assert!(view.turn_is_idle(), "reset clears the in-flight turn flag");
            assert!(view.plan.is_empty(), "reset clears the execution plan");
            assert!(
                view.fold
                    .is_expanded(&FoldKey::Thinking(1), FoldContext::past(true)),
                "reset drops fold overrides back to the mode's rule"
            );
            assert_eq!(
                view.fold.chosen_mode(),
                Some(FoldPreset::Expanded.mode()),
                "reset keeps the pane's fold mode: it is a user preference, not                  conversation state, and wiping it here also erases it from disk"
            );
        })
        .unwrap();
}

// Which mode a pane remembers as its own pick.

/// Only the user's pick is remembered: a mode the adapter reports — at connect
/// or by switching on its own — must not outrank the agent's `default_mode` on
/// the next connect.
#[gpui::test]
fn only_a_user_mode_pick_is_remembered(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{AcpEvent, ModeStateView, SessionCapabilitiesView, SessionModeView};

    let modes = |current: &str| ModeStateView {
        available: ["default", "plan", "bypassPermissions"]
            .into_iter()
            .map(|id| SessionModeView {
                id: id.into(),
                name: id.into(),
                description: None,
            })
            .collect(),
        current: current.into(),
    };
    let window = make_test_view(cx);
    window
        .update(cx, |v, _, cx| {
            v.apply_event(
                AcpEvent::Connected {
                    program: None,
                    session_id: "s1".into(),
                    modes: Some(modes("default")),
                    config_options: Vec::new(),
                    capabilities: SessionCapabilitiesView::default(),
                    login_methods: Vec::new(),
                },
                "",
                false,
                cx,
            );
            v.apply_event(
                AcpEvent::ModeChanged {
                    state: modes("plan"),
                },
                "",
                false,
                cx,
            );
            assert_eq!(
                v.picked_mode_id, None,
                "a mode the adapter reported is not a pick"
            );

            v.set_mode("bypassPermissions".into(), cx);
            v.apply_event(
                AcpEvent::ModeChanged {
                    state: modes("default"),
                },
                "",
                false,
                cx,
            );
            assert_eq!(
                v.picked_mode_id.as_deref(),
                Some("bypassPermissions"),
                "the adapter switching on its own leaves the user's pick standing"
            );
        })
        .unwrap();
}

#[gpui::test]
fn mode_state_updates_replace_and_survive_config_refresh(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{
        AcpEvent, ConfigChoiceView, ConfigOptionCategoryView, ConfigOptionView, ModeStateView,
        SessionModeView,
    };

    let mode = |id: &str| SessionModeView {
        id: id.to_string(),
        name: id.to_uppercase(),
        description: None,
    };

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.session_config.modes = Some(ModeStateView {
                available: vec![
                    SessionModeView {
                        id: "auto".to_string(),
                        name: "Auto".to_string(),
                        description: None,
                    },
                    SessionModeView {
                        id: "plan".to_string(),
                        name: "Plan".to_string(),
                        description: Some("Plan mode".to_string()),
                    },
                ],
                current: "auto".to_string(),
            });
            view.set_mode("plan".to_string(), cx);
            assert_eq!(
                view.session_config
                    .modes
                    .as_ref()
                    .expect("modes were injected")
                    .current,
                "plan",
                "set_mode flips current immediately (optimistic)"
            );

            // Connect-time state: the model in use advertises no `auto`.
            view.session_config.modes = Some(ModeStateView {
                available: vec![mode("default"), mode("plan")],
                current: "plan".to_string(),
            });
            // The agent switched to a model that supports `auto` and
            // re-advertised — list and current mode both move.
            view.apply_event(
                AcpEvent::ModeChanged {
                    state: ModeStateView {
                        available: vec![mode("auto"), mode("default"), mode("plan")],
                        current: "auto".to_string(),
                    },
                },
                "",
                false,
                cx,
            );
            let modes = view
                .session_config
                .modes
                .as_ref()
                .expect("modes stay advertised");
            assert_eq!(
                modes
                    .available
                    .iter()
                    .map(|m| m.id.as_str())
                    .collect::<Vec<_>>(),
                ["auto", "default", "plan"],
                "the rebuilt list replaces the connect-time one"
            );
            assert_eq!(
                modes.current, "auto",
                "a mode absent from the connect-time list still applies"
            );

            view.apply_event(
                AcpEvent::ConfigOptionsChanged(vec![ConfigOptionView {
                    id: "model".to_string(),
                    name: "Model".to_string(),
                    description: None,
                    category: ConfigOptionCategoryView::Model,
                    kind: daruda_acp::ConfigOptionKindView::Select {
                        current_value: "sonnet".to_string(),
                        options: vec![ConfigChoiceView {
                            value: "sonnet".to_string(),
                            name: "Sonnet".to_string(),
                            description: None,
                        }],
                    },
                }]),
                "",
                false,
                cx,
            );
            assert_eq!(
                view.session_config
                    .modes
                    .as_ref()
                    .expect("modes untouched")
                    .current,
                "auto"
            );
            assert_eq!(view.session_config.config_options.len(), 1);
        })
        .unwrap();
}

// Permission requests: routing a resolution by id and draining on Stop.

#[gpui::test]
fn permission_resolution_routes_by_id_reprojects_and_cancel_drains(cx: &mut gpui::TestAppContext) {
    use daruda_acp::{ChatItem, PermissionKindView, PermissionResolution, ToolStatusView};

    let window = make_test_view(cx);
    window
        .update(cx, |view, live_window, cx| {
            view.items = vec![permission_card(100), permission_card(200)];
            view.pending_permissions.insert(100);
            view.pending_permissions.insert(200);

            view.respond_permission(
                100,
                "allow_once".to_string(),
                PermissionKindView::AllowOnce,
                cx,
            );

            let ChatItem::Permission(first) = &view.items[0] else {
                panic!("expected first permission card");
            };
            let ChatItem::Permission(second) = &view.items[1] else {
                panic!("expected second permission card");
            };
            assert_eq!(
                first.resolved,
                Some(PermissionResolution::Chosen("allow_once".to_string())),
                "the answered card resolves by id"
            );
            assert_eq!(
                second.resolved, None,
                "answering one permission leaves the other live"
            );
            assert!(!view.is_permission_outstanding(100));
            assert!(view.is_permission_outstanding(200));

            view.respond_permission(
                200,
                "reject_once".to_string(),
                PermissionKindView::RejectOnce,
                cx,
            );
            let ChatItem::Permission(second) = &view.items[1] else {
                panic!("expected second permission card");
            };
            assert_eq!(
                second.resolved,
                Some(PermissionResolution::Chosen("reject_once".to_string()))
            );
            assert!(!view.has_pending_permission());

            view.items = vec![
                ChatItem::UserText("q".into()),
                tool_call("a", ToolStatusView::Completed, None),
                tool_call("b", ToolStatusView::Completed, None),
                permission_card(7),
            ];
            view.pending_permissions.insert(7);
            view.set_all_folds(false, live_window, cx);
            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::AgentItem(3)) && !r.hidden),
                "a pending permission stays visible under a collapsed response"
            );

            view.respond_permission(
                7,
                "allow_once".to_string(),
                PermissionKindView::AllowOnce,
                cx,
            );
            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.kind, RowKind::AgentItem(3)) && r.hidden),
                "a resolved permission folds back into the collapsed response immediately"
            );

            view.items = vec![
                permission_card(10),
                permission_card(20),
                permission_card(30),
            ];
            view.pending_permissions.extend([10, 20, 30]);
            view.set_turn_in_flight();
            view.cancel_turn(cx);

            // Stop also appends its marker, so match the cards by variant
            // rather than asserting every item is one.
            let cards: Vec<_> = view
                .items
                .iter()
                .filter_map(|item| match item {
                    ChatItem::Permission(card) => Some(card),
                    _ => None,
                })
                .collect();
            assert_eq!(cards.len(), 3, "expected the three permission cards");
            for card in cards {
                assert_eq!(
                    card.resolved,
                    Some(PermissionResolution::Cancelled),
                    "Stop cancels every outstanding permission"
                );
            }
            assert!(!view.has_pending_permission());
        })
        .unwrap();
}
