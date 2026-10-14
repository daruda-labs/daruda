use super::*;
use agent_client_protocol::schema::v1::{
    ContentChunk, InitializeResponse, NewSessionResponse, PromptResponse,
};

struct PendingPrompt {
    text: String,
    finish: oneshot::Sender<StopReason>,
}

struct Session {
    handle: AcpSessionHandle,
    events: UnboundedReceiver<AcpEvent>,
    prompts: UnboundedReceiver<PendingPrompt>,
    cancellations: UnboundedReceiver<SessionId>,
    task: smol::Task<Result<(), AcpClientError>>,
}

async fn within<T>(future: impl Future<Output = T>) -> T {
    with_connect_timeout("session lifecycle test", DISPATCH_TEST_TIMEOUT, async {
        Ok(future.await)
    })
    .await
    .expect("session lifecycle must make progress")
}

async fn start_session() -> Session {
    let (prompt_tx, prompts) = unbounded();
    let (cancel_tx, cancellations) = unbounded();
    let agent = Agent
        .builder()
        .on_receive_request(
            async |_: InitializeRequest, responder, _| {
                responder.respond(InitializeResponse::new(ProtocolVersion::V1))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |_: NewSessionRequest, responder, _| {
                responder.respond(NewSessionResponse::new("lifecycle"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, connection| {
                let [ContentBlock::Text(text)] = request.prompt.as_slice() else {
                    panic!("expected one text block");
                };
                let (finish, finished) = oneshot::channel();
                prompt_tx
                    .unbounded_send(PendingPrompt {
                        text: text.text.clone(),
                        finish,
                    })
                    .unwrap();
                let sender = connection.clone();
                // Keep dispatch free so cancellation arrives while the response is held.
                connection.spawn(async move {
                    let reason = finished.await.unwrap();
                    sender.send_notification(SessionNotification::new(
                        request.session_id,
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(
                            request.prompt.into_iter().next().unwrap(),
                        )),
                    ))?;
                    responder.respond(PromptResponse::new(reason))
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |notification: CancelNotification, _| {
                cancel_tx.unbounded_send(notification.session_id).unwrap();
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        );
    let (commands, command_rx) = unbounded();
    let (event_tx, mut events) = unbounded();
    let permission_parks = Arc::new(Mutex::new(HashMap::new()));
    let handle = AcpSessionHandle {
        commands,
        permission_parks: permission_parks.clone(),
    };
    let task = smol::spawn(run_connection(
        ClientCapabilitiesView::default(),
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
    loop {
        match within(events.next()).await.expect("connection event") {
            AcpEvent::Connected { .. } => break,
            AcpEvent::ConnectProgress(_) | AcpEvent::AgentIdentified { .. } => {}
            event => panic!("unexpected handshake event: {event:?}"),
        }
    }
    Session {
        handle,
        events,
        prompts,
        cancellations,
        task,
    }
}

async fn expect_turn(events: &mut UnboundedReceiver<AcpEvent>, text: &str, reason: StopReason) {
    match within(events.next()).await {
        Some(AcpEvent::Update(update)) => match *update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                assert_eq!(chunk.content, ContentBlock::Text(TextContent::new(text)));
            }
            update => panic!("unexpected update: {update:?}"),
        },
        event => panic!("expected final update before turn completion: {event:?}"),
    }
    match within(events.next()).await {
        Some(AcpEvent::TurnEnded {
            completed_normally,
            stop_reason,
            ..
        }) => {
            assert_eq!(completed_normally, reason != StopReason::Cancelled);
            assert_eq!(stop_reason, format!("{reason:?}"));
        }
        event => panic!("expected exactly one turn completion: {event:?}"),
    }
}

#[test]
fn cancellation_preserves_fifo_and_one_completion_per_prompt() {
    smol::block_on(async {
        let mut session = start_session().await;
        session.handle.send_prompt("first".into());
        let first = within(session.prompts.next()).await.unwrap();
        assert_eq!(first.text, "first");

        session.handle.send_prompt("second".into());
        session.handle.send_prompt("third".into());
        session.handle.cancel();
        assert_eq!(
            within(session.cancellations.next()).await,
            Some(SessionId::from("lifecycle"))
        );
        // Seeing cancel proves the commands ahead of it were serviced while blocked.
        assert!(session.prompts.try_recv().is_err());
        assert!(session.events.try_recv().is_err());

        first.finish.send(StopReason::Cancelled).unwrap();
        expect_turn(&mut session.events, "first", StopReason::Cancelled).await;
        for text in ["second", "third"] {
            let prompt = within(session.prompts.next()).await.unwrap();
            assert_eq!(prompt.text, text);
            prompt.finish.send(StopReason::EndTurn).unwrap();
            expect_turn(&mut session.events, text, StopReason::EndTurn).await;
        }

        drop(session.handle);
        within(session.task).await.expect("idle shutdown succeeds");
        assert!(within(session.events.next()).await.is_none());
        assert!(within(session.prompts.next()).await.is_none());
        assert!(within(session.cancellations.next()).await.is_none());
    });
}

#[test]
fn dropping_handle_mid_turn_waits_for_final_update_and_cancelled_response() {
    smol::block_on(async {
        let mut session = start_session().await;
        session.handle.send_prompt("last".into());
        let prompt = within(session.prompts.next()).await.unwrap();
        assert_eq!(prompt.text, "last");
        drop(session.handle);

        assert_eq!(
            within(session.cancellations.next()).await,
            Some(SessionId::from("lifecycle"))
        );
        assert!(session.events.try_recv().is_err());
        prompt.finish.send(StopReason::Cancelled).unwrap();
        expect_turn(&mut session.events, "last", StopReason::Cancelled).await;
        within(session.task)
            .await
            .expect("active shutdown succeeds");
        assert!(within(session.events.next()).await.is_none());
        assert!(within(session.cancellations.next()).await.is_none());
    });
}
