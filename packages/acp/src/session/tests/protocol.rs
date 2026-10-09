use super::*;

#[test]
fn required_resume_wire_never_sends_session_new() {
    use agent_client_protocol::{Channel, RawJsonRpcMessage, TransportFrame};
    use futures::SinkExt as _;
    for supports_load in [false, true] {
        let (transport, mut peer) = Channel::duplex();
        let (_commands, command_rx) = unbounded();
        let (event_tx, _events) = unbounded();
        smol::block_on(async {
            let peer_task = smol::spawn(async move {
                let mut methods = Vec::new();
                while let Some(TransportFrame::Single(RawJsonRpcMessage::Request(request))) =
                    peer.rx.next().await
                {
                    methods.push(request.method.to_string());
                    let response = match request.method.as_ref() {
                        "initialize" => Ok(
                            serde_json::json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": supports_load}}),
                        ),
                        "session/load" => Err(agent_client_protocol::Error::invalid_params()),
                        method => panic!("unexpected request during strict restore: {method}"),
                    };
                    peer.tx
                        .send(TransportFrame::Single(RawJsonRpcMessage::response(
                            request.id, response,
                        )))
                        .await
                        .unwrap();
                }
                methods
            });
            let result = run_connection(
                transport,
                PathBuf::from("."),
                None,
                Vec::new(),
                SessionResume::Required(SessionId::from("task-session")),
                Vec::new(),
                command_rx,
                event_tx,
                Arc::new(Mutex::new(HashMap::new())),
            )
            .await;
            assert!(result.is_err());
            let methods = peer_task.await;
            assert_eq!(
                methods,
                if supports_load {
                    vec!["initialize", "session/load"]
                } else {
                    vec!["initialize"]
                }
            );
        });
    }
}

#[test]
fn transport_eof_during_initialize_or_prompt_terminates_without_turn_failed() {
    use agent_client_protocol::{Channel, RawJsonRpcMessage, TransportFrame};
    use futures::SinkExt as _;
    for close_method in ["initialize", "session/load", "session/prompt"] {
        let (transport, mut peer) = Channel::duplex();
        let (command_tx, command_rx) = unbounded();
        command_tx
            .unbounded_send(Command::Prompt("hello".into()))
            .unwrap();
        let (event_tx, mut events) = unbounded();
        smol::block_on(async {
            let peer_task = smol::spawn(async move {
                while let Some(TransportFrame::Single(RawJsonRpcMessage::Request(request))) =
                    peer.rx.next().await
                {
                    if request.method.as_ref() == close_method {
                        return;
                    }
                    let response = match request.method.as_ref() {
                        "initialize" => {
                            serde_json::json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true}})
                        }
                        "session/new" => serde_json::json!({"sessionId": "saved-session"}),
                        method => panic!("unexpected request: {method}"),
                    };
                    peer.tx
                        .send(TransportFrame::Single(RawJsonRpcMessage::response(
                            request.id,
                            Ok(response),
                        )))
                        .await
                        .unwrap();
                }
            });
            let result = with_connect_timeout("EOF regression", Duration::from_secs(2), async {
                let resume =
                    (close_method == "session/load").then(|| SessionId::from("saved-session"));
                run_connection(
                    transport,
                    PathBuf::from("."),
                    None,
                    Vec::new(),
                    resume,
                    Vec::new(),
                    command_rx,
                    event_tx,
                    Arc::new(Mutex::new(HashMap::new())),
                )
                .await
                .map_err(|error| match error {
                    AcpClientError::Protocol(AcpFailure::TransportClosed { .. }) => {
                        agent_client_protocol::Error::internal_error().data(serde_json::json!({
                            "reason": agent_client_protocol::INCOMING_TRANSPORT_CLOSED_REASON
                        }))
                    }
                    other => panic!("unexpected error: {other}"),
                })
            })
            .await;
            assert!(agent_client_protocol::is_incoming_transport_closed(
                &result.unwrap_err()
            ));
            peer_task.await;
            while let Some(event) = events.next().await {
                assert!(!matches!(
                    event,
                    AcpEvent::TurnFailed(_) | AcpEvent::TurnEnded { .. }
                ));
            }
        });
    }
}

/// The field has always been in the schema and daruda never used it; this
/// pins that a requested server actually rides out on `session/new`.
#[test]
fn new_session_carries_the_requested_mcp_servers() {
    use agent_client_protocol::schema::v1::{EnvVariable, McpServerStdio};

    let server = McpServer::Stdio(
        McpServerStdio::new("daruda-abc", "/usr/local/bin/daruda")
            .args(vec!["--mcp".to_string()])
            .env(vec![EnvVariable::new(
                daruda_core::process_env::CONTROL_TOKEN.name(),
                "tok",
            )]),
    );
    let request = build_new_session_request(PathBuf::from("/tmp"), vec![server.clone()]);
    assert_eq!(request.mcp_servers, vec![server]);
    assert_eq!(request.cwd, PathBuf::from("/tmp"));
}

/// A resume must carry the servers too — see [`build_load_session_request`].
#[test]
fn a_resumed_session_carries_the_same_mcp_servers() {
    use agent_client_protocol::schema::v1::{EnvVariable, McpServerStdio};

    let server = McpServer::Stdio(
        McpServerStdio::new("daruda-abc", "/usr/local/bin/daruda")
            .args(vec![String::from("--mcp")])
            .env(vec![EnvVariable::new(
                daruda_core::process_env::CONTROL_TOKEN.name(),
                "tok",
            )]),
    );
    let request = build_load_session_request(
        SessionId::from("sess-1"),
        PathBuf::from("/tmp"),
        vec![server.clone()],
    );
    assert_eq!(request.mcp_servers, vec![server]);
    assert_eq!(request.session_id, SessionId::from("sess-1"));
    assert_eq!(request.cwd, PathBuf::from("/tmp"));
}

#[test]
fn no_requested_servers_leaves_the_list_empty() {
    let request = build_new_session_request(PathBuf::from("/tmp"), Vec::new());
    assert!(request.mcp_servers.is_empty());
}

#[test]
fn cancelled_is_not_a_normal_completion() {
    assert!(!turn_completed_normally(&StopReason::Cancelled));
}

#[test]
fn non_cancelled_stop_reasons_are_normal_completions() {
    assert!(turn_completed_normally(&StopReason::EndTurn));
    assert!(turn_completed_normally(&StopReason::MaxTokens));
    assert!(turn_completed_normally(&StopReason::MaxTurnRequests));
    assert!(turn_completed_normally(&StopReason::Refusal));
}

#[test]
fn allow_decision_maps_to_selected_outcome() {
    let resp = PermissionDecision::Allow {
        option_id: "allow_once".to_string(),
    }
    .into_response();
    match resp.outcome {
        RequestPermissionOutcome::Selected(sel) => {
            assert_eq!(sel.option_id, PermissionOptionId::from("allow_once"));
        }
        other => panic!("expected Selected, got {other:?}"),
    }
}

#[test]
fn reject_decision_also_maps_to_selected_outcome() {
    // Reject is a *selected* reject-kind option, not a protocol Cancelled.
    let resp = PermissionDecision::Reject {
        option_id: "reject_once".to_string(),
    }
    .into_response();
    match resp.outcome {
        RequestPermissionOutcome::Selected(sel) => {
            assert_eq!(sel.option_id, PermissionOptionId::from("reject_once"));
        }
        other => panic!("expected Selected, got {other:?}"),
    }
}

#[test]
fn cancelled_decision_maps_to_cancelled_outcome() {
    let resp = PermissionDecision::Cancelled.into_response();
    assert!(matches!(resp.outcome, RequestPermissionOutcome::Cancelled));
}

#[test]
fn respond_permission_unparks_the_waiting_handler() {
    let parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));
    let (command_tx, _command_rx) = unbounded::<Command>();
    let handle = AcpSessionHandle {
        commands: command_tx,
        permission_parks: parks.clone(),
    };

    let (decision_tx, decision_rx) = oneshot::channel();
    parks.lock().unwrap().insert(7, decision_tx);

    handle.respond_permission(
        7,
        PermissionDecision::Allow {
            option_id: "ok".to_string(),
        },
    );

    let received = smol::block_on(decision_rx).expect("sender was sent");
    assert_eq!(
        received,
        PermissionDecision::Allow {
            option_id: "ok".to_string()
        }
    );
    assert!(parks.lock().unwrap().is_empty(), "id must be consumed");
}

#[test]
fn info_field_change_maps_all_three_maybe_undefined_states() {
    use agent_client_protocol::schema::MaybeUndefined;
    assert_eq!(
        InfoFieldChange::from(MaybeUndefined::<String>::Undefined),
        InfoFieldChange::Unchanged
    );
    assert_eq!(
        InfoFieldChange::from(MaybeUndefined::<String>::Null),
        InfoFieldChange::Cleared
    );
    assert_eq!(
        InfoFieldChange::from(MaybeUndefined::Value("Fix parser bug".to_string())),
        InfoFieldChange::Set("Fix parser bug".to_string())
    );
}

#[test]
fn client_capabilities_withhold_terminal_output_and_the_terminal_methods() {
    // The adapter gates its shell-output sideband on the literal wire shape
    // `_meta.terminal_output === true`, so assert the serialized JSON, not
    // just the builder call. It stays withheld until a live wire capture
    // confirms the three-notification sequence the parser was written
    // against. `terminal` must stay off too: claiming it promises the
    // `terminal/*` methods this client does not implement.
    let json = serde_json::to_value(client_capabilities()).expect("caps serialize");
    assert_eq!(
        json.get("_meta").and_then(|m| m.get("terminal_output")),
        None,
        "the sideband flag is implemented but deliberately not advertised"
    );
    assert_ne!(
        json.get("terminal"),
        Some(&serde_json::Value::Bool(true)),
        "daruda implements no terminal/* method — advertising it would be a lie"
    );
}

/// `auth.terminal` is claimed even though top-level `terminal` is not, and
/// the two must not be conflated: the first only lets the agent *list*
/// terminal login methods, the second promises the `terminal/*` RPCs.
///
/// Asserted on the serialized shape because the agent gates on the literal
/// wire path `clientCapabilities.auth.terminal === true` — a builder call
/// that landed the flag anywhere else would read as "not advertised" and
/// silently return an empty `authMethods`, which is exactly the failure
/// this advertisement exists to end.
#[test]
fn client_capabilities_claim_native_subagent_sessions_in_the_shape_the_gate_reads() {
    // Pinned against the adapters' own check: an integer `version` of at
    // least 1 and an array `capabilities` containing the key. A shape that
    // merely looks right is silently ignored, and the symptom is an
    // invisible subagent rather than an error.
    let json = serde_json::to_value(client_capabilities()).expect("caps serialize");
    let air = &json["_meta"]["jetbrains"]["air"];
    assert_eq!(air["version"], serde_json::json!(1));
    assert_eq!(
        air["capabilities"],
        serde_json::json!(["nativeSubagentSessions"])
    );
}

#[test]
fn client_capabilities_claim_terminal_auth_without_claiming_terminal_methods() {
    let json = serde_json::to_value(client_capabilities()).expect("caps serialize");
    assert_eq!(
        json.get("auth").and_then(|a| a.get("terminal")),
        Some(&serde_json::Value::Bool(true)),
        "without this the agent advertises no login methods at all"
    );
    assert_ne!(
        json.get("terminal"),
        Some(&serde_json::Value::Bool(true)),
        "auth.terminal must not drag in the terminal/* promise"
    );
    // The companion flag is read at the ROOT, not under `auth`. Nesting it
    // beside `auth.terminal` reads as correct, serializes fine, and is
    // silently ignored — a live capture caught exactly that.
    assert_eq!(
        json.get("_meta")
            .and_then(|m| m.get(super::TERMINAL_AUTH_META_KEY)),
        Some(&serde_json::Value::Bool(true)),
        "terminal-auth belongs on clientCapabilities._meta, not auth._meta"
    );
    assert_eq!(
        json.get("auth").and_then(|a| a.get("_meta")),
        None,
        "the agent never looks here"
    );
}

#[test]
fn session_capabilities_reads_advertised_flags() {
    use agent_client_protocol::schema::v1::{
        AgentCapabilities, SessionCapabilities, SessionCloseCapabilities, SessionResumeCapabilities,
    };
    // Advertise load (top-level bool) + resume + close; leave list/fork off.
    let caps = AgentCapabilities::new()
        .load_session(true)
        .session_capabilities(
            SessionCapabilities::new()
                .resume(SessionResumeCapabilities::new())
                .close(SessionCloseCapabilities::new()),
        );
    let v = session_capabilities_from_protocol(&caps);
    assert!(v.load, "load_session bool must map to load");
    assert!(v.resume, "advertised resume must map");
    assert!(v.close, "advertised close must map");
    assert!(!v.list, "unadvertised list must be false");
}

#[test]
fn usage_view_maps_tokens_and_cost() {
    use agent_client_protocol::schema::v1::{Cost, UsageUpdate};
    let u = UsageUpdate::new(53_000, 200_000).cost(Cost::new(0.045, "USD"));
    let v = crate::model::UsageView::from(&u);
    assert_eq!(v.used, 53_000);
    assert_eq!(v.size, 200_000);
    let cost = v.cost.expect("cost must map when present");
    assert_eq!(cost.currency, "USD");
    assert!((cost.amount - 0.045).abs() < f64::EPSILON);
}

#[test]
fn usage_view_without_cost_is_none() {
    use agent_client_protocol::schema::v1::UsageUpdate;
    let v = crate::model::UsageView::from(&UsageUpdate::new(10, 100));
    assert!(v.cost.is_none(), "absent cost must stay None");
}

#[test]
fn resolve_resume_loads_when_supported() {
    let id = SessionId::from("sess-1");
    let (to_load, notice) = resolve_resume(Some(id.clone()), true);
    assert_eq!(to_load, Some(id));
    assert!(notice.is_none(), "supported resume needs no notice");
}

#[test]
fn required_resume_never_downgrades_to_new_session() {
    let id = SessionId::from("task-session");
    assert!(SessionResume::Required(id.clone()).resolve(false).is_err());
    let (session, notice) = SessionResume::Required(id.clone()).resolve(true).unwrap();
    assert_eq!(session, Some(id));
    assert!(notice.is_none());
}

#[test]
fn resolve_resume_downgrades_to_fresh_when_load_unsupported() {
    let (to_load, notice) = resolve_resume(Some(SessionId::from("sess-1")), false);
    assert!(to_load.is_none(), "must start fresh when load unsupported");
    assert!(notice.is_some(), "downgrade must advise the user");
}

#[test]
fn resolve_resume_fresh_session_is_silent() {
    let (to_load, notice) = resolve_resume(None, true);
    assert!(to_load.is_none());
    assert!(notice.is_none(), "a plain fresh session is not a downgrade");
}

#[test]
fn session_capabilities_default_agent_advertises_nothing() {
    use agent_client_protocol::schema::v1::AgentCapabilities;
    let v = session_capabilities_from_protocol(&AgentCapabilities::new());
    assert_eq!(v, SessionCapabilitiesView::default());
}

#[test]
fn config_options_from_protocol_maps_select_options() {
    use agent_client_protocol::schema::v1::{
        SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
    };
    let opts = vec![
        SessionConfigOption::select(
            "model",
            "Model",
            "sonnet",
            vec![SessionConfigSelectOption::new("sonnet", "Sonnet")],
        )
        .category(SessionConfigOptionCategory::Model),
    ];
    let views = config_options_from_protocol(&opts);
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].id, "model");
    assert!(matches!(
        &views[0].kind,
        crate::model::ConfigOptionKindView::Select { current_value, .. }
            if current_value == "sonnet"
    ));
    assert_eq!(
        views[0].category,
        crate::model::ConfigOptionCategoryView::Model
    );
}

#[test]
fn respond_permission_for_unknown_id_is_a_noop() {
    let parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));
    let (command_tx, _command_rx) = unbounded::<Command>();
    let handle = AcpSessionHandle {
        commands: command_tx,
        permission_parks: parks,
    };
    // Must not panic.
    handle.respond_permission(99, PermissionDecision::Cancelled);
}

#[test]
fn with_connect_timeout_passes_through_a_future_that_resolves_first() {
    let result = smol::block_on(with_connect_timeout(
        "probe",
        Duration::from_secs(5),
        std::future::ready(Ok::<_, agent_client_protocol::Error>(42)),
    ));
    assert_eq!(result.unwrap(), 42);
}

#[test]
fn with_connect_timeout_errors_out_a_hung_future() {
    // A future that never resolves models a stuck handshake (e.g. an
    // SSH-wrapped adapter blocked on a silent auth prompt): without the
    // race against the timer, `run_connection` would await this forever
    // and the host would never see an `AcpEvent`, matching the reported
    // "stuck on Connecting" symptom.
    let result = smol::block_on(with_connect_timeout(
        "probe",
        Duration::from_millis(20),
        std::future::pending::<Result<u32, agent_client_protocol::Error>>(),
    ));
    let err = result.expect_err("a never-resolving future must time out");
    assert!(err.message.contains("probe"), "{}", err.message);
    assert!(err.message.contains("timed out"), "{}", err.message);
}
