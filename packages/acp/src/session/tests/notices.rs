use super::*;
use agent_client_protocol::schema::v1::{InitializeResponse, NewSessionResponse, PromptResponse};

#[test]
fn notices_are_opt_in_without_enabling_other_presentation_contracts() {
    for notices in [false, true] {
        let caps =
            serde_json::to_value(client_capabilities(ClientCapabilitiesView { notices })).unwrap();
        assert_eq!(caps["session"]["notices"].is_object(), notices);
        assert!(caps["session"].get("compaction").is_none());
        assert!(caps.get("subagents").is_none());
        assert!(caps["session"]["configOptions"]["boolean"].is_object());
    }
}

#[test]
fn protocol_notices_preserve_delivery_order_without_ending_the_turn() {
    smol::block_on(async {
        let agent = Agent
            .builder()
            .on_receive_request(
                async |request: InitializeRequest, responder, _| {
                    assert!(
                        request
                            .client_capabilities
                            .session
                            .unwrap()
                            .notices
                            .is_some()
                    );
                    responder.respond(InitializeResponse::new(ProtocolVersion::V1))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |_: NewSessionRequest, responder, _| {
                    responder.respond(NewSessionResponse::new("notices"))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |request: PromptRequest, responder, connection| {
                    let ContentBlock::Text(text) = &request.prompt[0] else {
                        panic!("expected text prompt");
                    };
                    let child = format!("child-{}", text.text);
                    let grandchild = format!("grandchild-{}", text.text);
                    for (parent, child) in [
                        (request.session_id.clone(), child.clone()),
                        (SessionId::from(child.clone()), grandchild.clone()),
                    ] {
                        connection.send_notification(CompatSessionNotification {
                            session_id: parent,
                            update: serde_json::json!({
                                "sessionUpdate": "subagent_spawned",
                                "subagentSessionId": child,
                                "name": "Worker"
                            }),
                            meta: None,
                        })?;
                    }
                    for (session, severity) in [
                        (request.session_id.to_string(), "info"),
                        (child.clone(), "warning"),
                        (grandchild.clone(), "error"),
                        (grandchild, "future"),
                        (child, "error"),
                    ] {
                        connection.send_notification(CompatSessionNotification {
                            session_id: session.into(),
                            update: serde_json::json!({
                                "sessionUpdate": "notice",
                                "severity": severity,
                                "title": "Same title",
                                "description": "**Plain** detail"
                            }),
                            meta: None,
                        })?;
                    }
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                },
                agent_client_protocol::on_receive_request!(),
            );
        let (commands, command_rx) = unbounded();
        let (event_tx, mut events) = unbounded();
        let task = smol::spawn(run_connection(
            ClientCapabilitiesView { notices: true },
            agent,
            PathBuf::from("."),
            None,
            Vec::new(),
            None,
            Vec::new(),
            command_rx,
            event_tx,
            Arc::new(Mutex::new(HashMap::new())),
        ));
        loop {
            match next_event_within(&mut events)
                .await
                .expect("handshake event")
            {
                AcpEvent::Connected { .. } => break,
                AcpEvent::ConnectProgress(_) | AcpEvent::AgentIdentified { .. } => {}
                event => panic!("unexpected handshake event: {event:?}"),
            }
        }
        // Reuse the session: notices, including errors, must not close it.
        for turn in 0..2 {
            commands
                .unbounded_send(Command::Prompt(turn.to_string().into()))
                .unwrap();
            for child in [format!("child-{turn}"), format!("grandchild-{turn}")] {
                let Some(AcpEvent::Update(update)) = next_event_within(&mut events).await else {
                    panic!("expected a child announcement");
                };
                let SessionUpdate::ToolCall(call) = *update else {
                    panic!("expected a synthetic child tool call");
                };
                assert_eq!(call.tool_call_id.to_string(), format!("subagent:{child}"));
            }
            for (expected_session, severity) in [
                ("notices".to_owned(), "info"),
                (format!("child-{turn}"), "warning"),
                (format!("grandchild-{turn}"), "error"),
                (format!("grandchild-{turn}"), "future"),
                (format!("child-{turn}"), "error"),
            ] {
                let Some(AcpEvent::SessionNotice { session_id, notice }) =
                    next_event_within(&mut events).await
                else {
                    panic!("expected a notice, not transcript content or turn completion");
                };
                assert_eq!(session_id, expected_session);
                assert_eq!(notice.severity.as_str(), severity);
                assert_eq!(notice.title, "Same title");
                assert_eq!(notice.description.as_deref(), Some("**Plain** detail"));
            }
            assert!(matches!(
                next_event_within(&mut events).await,
                Some(AcpEvent::TurnEnded {
                    completed_normally: true,
                    ..
                })
            ));
        }
        drop(commands);
        with_connect_timeout("notice shutdown", DISPATCH_TEST_TIMEOUT, async {
            task.await.expect("connection succeeds");
            Ok(())
        })
        .await
        .unwrap();
        assert!(events.next().await.is_none());
    });
}
