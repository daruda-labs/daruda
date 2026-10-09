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
