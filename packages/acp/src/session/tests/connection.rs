use super::*;

/// A model switch can replace the mode vocabulary. The handshake must use
/// that replacement before choosing the configured mode, expose only the
/// settled state in `Connected`, and only then drain a queued prompt.
#[test]
fn initial_model_settles_before_mode_and_connected() {
    use agent_client_protocol::schema::v1::{
        InitializeResponse, NewSessionResponse, PromptResponse, SessionConfigOption,
        SessionConfigOptionCategory, SessionConfigSelectOption, SessionMode, SessionModeState,
        SetSessionConfigOptionResponse, SetSessionModeResponse,
    };

    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let agent = Agent
        .builder()
        .on_receive_request(
            async |_req: InitializeRequest, responder, _conn| {
                responder.respond(InitializeResponse::new(ProtocolVersion::V1))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |_req: NewSessionRequest, responder, _conn| {
                responder.respond(
                    NewSessionResponse::new("sess-model-mode-test")
                        .modes(SessionModeState::new(
                            "default",
                            vec![SessionMode::new("default", "Default")],
                        ))
                        .config_options(vec![
                            SessionConfigOption::select(
                                "model",
                                "Model",
                                "sonnet",
                                vec![
                                    SessionConfigSelectOption::new("sonnet", "Sonnet"),
                                    SessionConfigSelectOption::new("opus", "Opus"),
                                ],
                            )
                            .category(SessionConfigOptionCategory::Model),
                        ]),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let requests = requests.clone();
                async move |req: SetSessionConfigOptionRequest, responder, _conn| {
                    let model = req
                        .value
                        .as_value_id()
                        .expect("model uses a value id")
                        .to_string();
                    requests.lock().unwrap().push(format!("model:{model}"));
                    responder.respond(SetSessionConfigOptionResponse::new(vec![
                        SessionConfigOption::select(
                            "model",
                            "Model",
                            model,
                            vec![
                                SessionConfigSelectOption::new("sonnet", "Sonnet"),
                                SessionConfigSelectOption::new("opus", "Opus"),
                            ],
                        )
                        .category(SessionConfigOptionCategory::Model),
                        SessionConfigOption::select(
                            "mode",
                            "Mode",
                            "review",
                            vec![
                                SessionConfigSelectOption::new("review", "Review"),
                                SessionConfigSelectOption::new("plan", "Plan"),
                            ],
                        )
                        .category(SessionConfigOptionCategory::Mode),
                    ]))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let requests = requests.clone();
                async move |req: SetSessionModeRequest, responder, _conn| {
                    requests
                        .lock()
                        .unwrap()
                        .push(format!("mode:{}", req.mode_id));
                    responder.respond(SetSessionModeResponse::new())
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let requests = requests.clone();
                async move |_req: PromptRequest, responder, _conn| {
                    requests.lock().unwrap().push("prompt".to_string());
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                }
            },
            agent_client_protocol::on_receive_request!(),
        );

    let (command_tx, command_rx) = unbounded::<Command>();
    command_tx
        .unbounded_send(Command::Prompt("queued during connect".into()))
        .unwrap();
    let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
    let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));

    smol::block_on(async {
        let connection = smol::spawn(run_connection(
            agent,
            PathBuf::from("."),
            Some("opus".to_string()),
            vec!["plan".to_string()],
            None,
            Vec::new(),
            command_rx,
            event_tx,
            permission_parks,
        ));

        let (modes, config_options) = loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::Connected {
                    modes,
                    config_options,
                    ..
                }) => {
                    break (
                        modes.expect("model response advertised modes"),
                        config_options,
                    );
                }
                Some(_) => {}
                None => panic!("connection never reached Connected"),
            }
        };

        assert_eq!(modes.current, "plan");
        assert_eq!(
            modes
                .available
                .iter()
                .map(|mode| mode.id.as_str())
                .collect::<Vec<_>>(),
            ["review", "plan"]
        );
        assert!(
            config_options
                .iter()
                .all(|option| option.category != ConfigOptionCategoryView::Mode),
            "the mode option is represented only through Connected.modes"
        );
        let model = config_options
            .iter()
            .find(|option| option.category == ConfigOptionCategoryView::Model)
            .expect("model option remains advertised");
        assert!(matches!(
            &model.kind,
            ConfigOptionKindView::Select { current_value, .. } if current_value == "opus"
        ));

        loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::TurnEnded { .. }) => break,
                Some(_) => {}
                None => panic!("queued prompt never completed"),
            }
        }
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            ["model:opus", "mode:plan", "prompt"]
        );

        drop(command_tx);
        let _ = connection.await;
    });
}

/// A resume reports the adapter's own mode (`claude-agent-acp` recomputes
/// it per launch); the host's requested mode still has to win.
#[test]
fn a_resumed_session_applies_the_requested_mode() {
    use agent_client_protocol::schema::v1::{
        InitializeResponse, LoadSessionResponse, SessionMode, SessionModeState,
        SetSessionModeResponse,
    };

    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let agent = Agent
        .builder()
        .on_receive_request(
            async |_req: InitializeRequest, responder, _conn| {
                responder.respond(
                    InitializeResponse::new(ProtocolVersion::V1).agent_capabilities(
                        agent_client_protocol::schema::v1::AgentCapabilities::new()
                            .load_session(true),
                    ),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |_req: LoadSessionRequest, responder, _conn| {
                responder.respond(LoadSessionResponse::new().modes(SessionModeState::new(
                    "default",
                    vec![
                        SessionMode::new("default", "Default"),
                        SessionMode::new("bypassPermissions", "Bypass"),
                    ],
                )))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let requests = requests.clone();
                async move |req: SetSessionModeRequest, responder, _conn| {
                    requests.lock().unwrap().push(req.mode_id.to_string());
                    responder.respond(SetSessionModeResponse::new())
                }
            },
            agent_client_protocol::on_receive_request!(),
        );

    let (command_tx, command_rx) = unbounded::<Command>();
    let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
    let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));

    smol::block_on(async {
        let connection = smol::spawn(run_connection(
            agent,
            PathBuf::from("."),
            None,
            vec!["bypassPermissions".to_string()],
            Some(SessionId::from("sess-resumed")),
            Vec::new(),
            command_rx,
            event_tx,
            permission_parks,
        ));

        let modes = loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::Connected { modes, .. }) => {
                    break modes.expect("the load advertised modes");
                }
                Some(_) => {}
                None => panic!("connection never reached Connected"),
            }
        };
        assert_eq!(modes.current, "bypassPermissions");
        assert_eq!(requests.lock().unwrap().as_slice(), ["bypassPermissions"]);

        drop(command_tx);
        let _ = connection.await;
    });
}

/// Fake agent that answers the handshake as `program` (or anonymously) and,
/// on `session/prompt`, sends one `usage_update` carrying `meta` before
/// ending the turn.
fn usage_update_agent(
    program: Option<&'static str>,
    meta: serde_json::Value,
) -> impl ConnectTo<Client> {
    use agent_client_protocol::schema::v1::{
        Implementation, InitializeResponse, NewSessionResponse, PromptResponse, UsageUpdate,
    };
    let meta = meta.as_object().cloned();
    Agent
        .builder()
        .on_receive_request(
            async move |_req: InitializeRequest, responder, _conn| {
                let response = InitializeResponse::new(ProtocolVersion::V1);
                responder.respond(match program {
                    Some(name) => response.agent_info(Implementation::new(name, "0.0.0")),
                    None => response,
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |_req: NewSessionRequest, responder, _conn| {
                responder.respond(NewSessionResponse::new("sess-usage-test"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: PromptRequest, responder, conn| {
                conn.send_notification(SessionNotification::new(
                    req.session_id,
                    SessionUpdate::UsageUpdate(
                        UsageUpdate::new(33_813, 1_000_000).meta(meta.clone()),
                    ),
                ))?;
                responder.respond(PromptResponse::new(StopReason::EndTurn))
            },
            agent_client_protocol::on_receive_request!(),
        )
}

/// The first `UsageChanged` a session against `agent` emits for one prompt.
fn first_usage_from(agent: impl ConnectTo<Client> + 'static) -> crate::model::UsageView {
    let (command_tx, command_rx) = unbounded::<Command>();
    command_tx
        .unbounded_send(Command::Prompt("hi".into()))
        .unwrap();
    let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
    let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));
    smol::block_on(async {
        let connection = smol::spawn(run_connection(
            agent,
            PathBuf::from("."),
            None,
            Vec::new(),
            None,
            Vec::new(),
            command_rx,
            event_tx,
            permission_parks,
        ));
        let usage = loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::UsageChanged(usage)) => break usage,
                Some(_) => {}
                None => panic!("the usage update never reached the host"),
            }
        };
        drop(command_tx);
        let _ = connection.await;
        usage
    })
}

/// The wire-captured `_meta` of a usage update billed as overage.
fn billed_usage_meta() -> serde_json::Value {
    serde_json::json!({
        "_claude/rateLimit": {
            "status": "rejected",
            "resetsAt": 1791340200,
            "rateLimitType": "five_hour",
            "overageStatus": "allowed",
            "isUsingOverage": true
        },
        "_claude/model": "claude-opus-5-5"
    })
}

#[test]
fn a_claude_usage_update_reports_overage_billing() {
    let usage = first_usage_from(usage_update_agent(
        Some("@agentclientprotocol/claude-agent-acp"),
        billed_usage_meta(),
    ));
    assert_eq!((usage.used, usage.size), (33_813, 1_000_000));
    let rate_limit = usage.rate_limit.expect("the session read the rate limit");
    assert!(rate_limit.bills_overage());
    assert_eq!(
        rate_limit.window,
        Some(crate::model::RateLimitWindow::FiveHour)
    );
}

/// The report is read under the program's dialect, so the same `_meta` from
/// codex carries no standing.
#[test]
fn a_codex_usage_update_carries_no_rate_limit() {
    let usage = first_usage_from(usage_update_agent(Some("codex-acp"), billed_usage_meta()));
    assert_eq!(usage.rate_limit, None);
}
