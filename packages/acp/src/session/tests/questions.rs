//! Questions the agent asks mid-turn: claimed only when the host renders
//! them, parked until answered, and answered for it when it does not.

use super::*;
use crate::question::{QuestionAnswer, QuestionValue};

/// A fake agent that, on each prompt, asks one form question, then echoes
/// the client's response as message text and ends the turn — so a test reads
/// what the agent received from the event stream.
fn asking_agent() -> impl ConnectTo<Client> + 'static {
    use agent_client_protocol::schema::v1::{
        ContentChunk, ElicitationFormMode, ElicitationPropertySchema, ElicitationSchema,
        ElicitationSessionScope, EnumOption, InitializeResponse, NewSessionResponse,
        PromptResponse, StringPropertySchema,
    };
    Agent
        .builder()
        .on_receive_request(
            async |_req: InitializeRequest, responder, _conn| {
                responder.respond(InitializeResponse::new(ProtocolVersion::V1))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |_req: NewSessionRequest, responder, _conn| {
                responder.respond(NewSessionResponse::new("sess-question-test"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |req: PromptRequest, responder, conn| {
                let session_id = req.session_id.clone();
                let task_conn = conn.clone();
                conn.spawn(async move {
                    let mut select = StringPropertySchema::new();
                    select.one_of = Some(vec![
                        EnumOption::new("red", "red"),
                        EnumOption::new("blue", "blue"),
                    ]);
                    let mut schema = ElicitationSchema::new();
                    schema.properties.insert(
                        "question_0".into(),
                        ElicitationPropertySchema::String(select),
                    );
                    let scope = ElicitationSessionScope::new(session_id.clone()).tool_call_id(
                        agent_client_protocol::schema::v1::ToolCallId::new("toolu_1"),
                    );
                    let request = CreateElicitationRequest::new(
                        ElicitationFormMode::new(scope, schema),
                        "Which color?",
                    );
                    let reply = task_conn.send_request(request).block_task().await?;
                    let echoed = serde_json::to_string(&reply).expect("response serializes");
                    task_conn.send_notification(SessionNotification::new(
                        session_id,
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new(echoed),
                        ))),
                    ))?;
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
}

/// What the agent received, read back from its echo, and whether the host
/// was shown the question on the way.
fn ask_once(
    capabilities: ClientCapabilitiesView,
    answer: impl Fn(u64, &AcpSessionHandle) + 'static,
) -> (bool, serde_json::Value) {
    let (command_tx, command_rx) = unbounded::<Command>();
    let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
    let parks = Parks::default();
    let handle = AcpSessionHandle {
        commands: command_tx,
        parks: parks.clone(),
    };
    smol::block_on(async move {
        let connection = smol::spawn(run_connection(
            capabilities,
            asking_agent(),
            PathBuf::from("."),
            None,
            Vec::new(),
            None,
            Vec::new(),
            command_rx,
            event_tx,
            parks,
        ));
        loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::Connected { .. }) => break,
                Some(_) => {}
                None => panic!("connection never reached Connected"),
            }
        }
        handle.send_prompt("ask me".to_string());
        let mut asked = false;
        let echoed = loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::QuestionAsked { id, question }) => {
                    asked = true;
                    assert_eq!(question.message, "Which color?");
                    assert_eq!(question.tool_call_id.as_deref(), Some("toolu_1"));
                    answer(id, &handle);
                }
                Some(AcpEvent::Update(update)) => {
                    if let SessionUpdate::AgentMessageChunk(chunk) = *update
                        && let ContentBlock::Text(text) = chunk.content
                    {
                        break serde_json::from_str(&text.text).expect("echo is json");
                    }
                }
                Some(_) => {}
                None => panic!("the agent never echoed its reply"),
            }
        };
        drop(handle);
        let _ = connection.await;
        (asked, echoed)
    })
}

#[test]
fn a_host_that_renders_questions_answers_them() {
    let capabilities = ClientCapabilitiesView {
        questions: true,
        ..ClientCapabilitiesView::default()
    };
    let (asked, reply) = ask_once(capabilities, |id, handle| {
        let mut values = std::collections::BTreeMap::new();
        values.insert("question_0".into(), QuestionValue::Text("blue".into()));
        handle.respond_question(id, QuestionAnswer::Submit(values));
    });
    assert!(asked, "the question reached the host");
    assert_eq!(
        reply,
        serde_json::json!({ "action": "accept", "content": { "question_0": "blue" } })
    );
}

/// A host that never claimed questions is never shown one; the agent is
/// declined at once instead of left waiting on nobody.
#[test]
fn a_host_that_does_not_render_questions_declines_them() {
    let (asked, reply) = ask_once(ClientCapabilitiesView::default(), |_, _| {
        panic!("a host that did not claim questions must not be asked one")
    });
    assert!(!asked);
    assert_eq!(reply, serde_json::json!({ "action": "decline" }));
}

/// The host going away with a question open cancels it rather than leaving
/// the agent's turn waiting.
#[test]
fn a_dropped_question_is_cancelled() {
    let capabilities = ClientCapabilitiesView {
        questions: true,
        ..ClientCapabilitiesView::default()
    };
    let (_, reply) = ask_once(capabilities, |id, handle| {
        handle
            .parks
            .questions
            .lock()
            .unwrap()
            .remove(&id)
            .expect("parked");
    });
    assert_eq!(reply, serde_json::json!({ "action": "cancel" }));
}

#[test]
fn form_elicitation_is_claimed_only_for_a_host_that_renders_questions() {
    let caps = |questions| {
        serde_json::to_value(client_capabilities(ClientCapabilitiesView {
            questions,
            ..ClientCapabilitiesView::default()
        }))
        .expect("caps serialize")
    };
    assert_eq!(caps(false).get("elicitation"), None);
    assert_eq!(
        caps(true).get("elicitation"),
        Some(&serde_json::json!({ "form": {} })),
        "form only: a URL elicitation has nowhere to go in this host"
    );
}
