mod connection;
mod dispatch;
mod protocol;

use super::*;
use crate::model::ChatItem;
use agent_client_protocol::schema::v1::{PermissionOptionId, SessionNotification};

/// Ceiling for each awaited event in the dispatch-concurrency test. Big
/// enough that a loaded machine cannot fake a "blocked" verdict — the
/// failure mode under measurement is *never*, not *slow* (both endpoints
/// live in this process; no subprocess or network is involved).
const DISPATCH_TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// The next event, or `None` if none arrives within
/// [`DISPATCH_TEST_TIMEOUT`] — so a blocked dispatch loop reads as a test
/// failure instead of a hang.
async fn next_event_within(rx: &mut UnboundedReceiver<AcpEvent>) -> Option<AcpEvent> {
    // ALLOW: same rationale as `with_connect_timeout` — this GPUI-free
    // crate has no BackgroundExecutor to time on. The timer is only the
    // failure ceiling: on the passing path every awaited event arrives
    // immediately and the timer is dropped unpolled.
    #[allow(clippy::disallowed_methods)]
    let timer = smol::Timer::after(DISPATCH_TEST_TIMEOUT);
    futures::select! {
        event = rx.next().fuse() => event,
        // `Timer` is both a Future and a Stream; pick the Future fuse.
        _ = futures::FutureExt::fuse(timer) => None,
    }
}

/// In-process fake agent for the dispatch-concurrency measurement: answers
/// the handshake, and on `session/prompt` sends a permission request
/// IMMEDIATELY followed by a `session/update` notification — both enqueued
/// back to back on one outgoing queue, so the client is guaranteed to
/// receive them in that order. The turn ends only after the host's
/// permission decision arrives.
///
/// The agent's own prompt work runs via `conn.spawn` (not inline in the
/// handler) because awaiting the permission decision inside the agent's
/// handler would block the agent's *own* dispatch loop — the very defect
/// class the client side is being measured for.
fn permission_then_update_agent() -> impl ConnectTo<Client> + 'static {
    use agent_client_protocol::schema::v1::{
        ContentChunk, InitializeResponse, NewSessionResponse, PermissionOption,
        PermissionOptionKind, PromptResponse, ToolCallUpdate, ToolCallUpdateFields,
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
                responder.respond(NewSessionResponse::new("sess-dispatch-test"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |req: PromptRequest, responder, conn| {
                let session_id = req.session_id.clone();
                let task_conn = conn.clone();
                conn.spawn(async move {
                    let decision = task_conn.send_request(RequestPermissionRequest::new(
                        session_id.clone(),
                        ToolCallUpdate::new("tool-1", ToolCallUpdateFields::default()),
                        vec![PermissionOption::new(
                            "allow",
                            "Allow",
                            PermissionOptionKind::AllowOnce,
                        )],
                    ));
                    task_conn.send_notification(SessionNotification::new(
                        session_id,
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new("mid-permission update"),
                        ))),
                    ))?;
                    let _ = decision.block_task().await?;
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
}

/// In-process fake agent running the native subagent sequence: it announces
/// a child on the root session, sends the child's own tool call under the
/// *child's* session id, then reports the child completed.
///
/// Sent as [`CompatSessionNotification`] because that is the only way to put
/// a draft-protocol `sessionUpdate` on the wire — the typed
/// `SessionNotification` has no variant for one.
/// The three notifications a native subagent run produces: the spawn, one
/// child tool call under the *child's* session id, and the terminal state.
/// Shared so the live path and the resume path cannot drift apart.
fn native_subagent_replay(root: SessionId) -> [(SessionId, serde_json::Value); 3] {
    [
        (
            root.clone(),
            serde_json::json!({
                "sessionUpdate": "subagent_spawned",
                "subagentSessionId": "sess-kid",
                "name": "Lorentz",
                "task": "Probe the UI",
                "capabilities": {},
            }),
        ),
        (
            SessionId::from("sess-kid"),
            serde_json::json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "c1",
                "title": "Read main.rs",
                "kind": "read",
                "status": "completed",
            }),
        ),
        (
            root,
            serde_json::json!({
                "sessionUpdate": "subagent_state_update",
                "subagentSessionId": "sess-kid",
                "state": "completed",
            }),
        ),
    ]
}

fn native_subagent_agent() -> impl ConnectTo<Client> + 'static {
    use agent_client_protocol::schema::v1::{
        InitializeResponse, NewSessionResponse, PromptResponse,
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
                responder.respond(NewSessionResponse::new("sess-root"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |req: PromptRequest, responder, conn| {
                let root = req.session_id.clone();
                let unknown = (
                    // An update kind this build has no knowledge of, to prove
                    // it is reported rather than fatal.
                    root.clone(),
                    serde_json::json!({ "sessionUpdate": "quantum_update" }),
                );
                for (session, update) in native_subagent_replay(root).into_iter().chain([unknown]) {
                    conn.send_notification(CompatSessionNotification {
                        session_id: session,
                        update,
                        meta: None,
                    })?;
                }
                responder.respond(PromptResponse::new(StopReason::EndTurn))
            },
            agent_client_protocol::on_receive_request!(),
        )
}

/// Replays a native subagent exchange *during* `session/load`, before the
/// load response returns. `LoadSessionResponse` is what lets the host reach
/// `Connected`, so every notification here lands while the session is still
/// resuming.
fn native_subagent_resume_agent() -> impl ConnectTo<Client> + 'static {
    use agent_client_protocol::schema::v1::{InitializeResponse, LoadSessionResponse};
    Agent
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
            async |req: LoadSessionRequest, responder, conn| {
                for (session, update) in native_subagent_replay(req.session_id.clone()) {
                    conn.send_notification(CompatSessionNotification {
                        session_id: session,
                        update,
                        meta: None,
                    })?;
                }
                responder.respond(LoadSessionResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
}

/// An agent that delegates the legacy way, and stamps the collaboration
/// marker on an update this build cannot type *before* the one it can.
/// That order is the point: only the typable one reaches the arm that
/// emits, so the notice has to survive the one ahead of it.
fn legacy_delegation_agent() -> impl ConnectTo<Client> + 'static {
    use agent_client_protocol::schema::v1::{
        InitializeResponse, NewSessionResponse, PromptResponse,
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
                responder.respond(NewSessionResponse::new("sess-root"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |req: PromptRequest, responder, conn| {
                let root = req.session_id.clone();
                let marker = |tool: &str| {
                    serde_json::json!({ "codex": { "collaboration": { "tool": tool } } })
                };
                let updates = [
                    serde_json::json!({
                        "sessionUpdate": "quantum_update",
                        "_meta": marker("spawnAgent"),
                    }),
                    serde_json::json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": "t1",
                        "title": "spawnAgent",
                        "_meta": marker("spawnAgent"),
                    }),
                    serde_json::json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": "t2",
                        "title": "wait",
                        "_meta": marker("wait"),
                    }),
                ];
                for update in updates {
                    conn.send_notification(CompatSessionNotification {
                        session_id: root.clone(),
                        update,
                        meta: None,
                    })?;
                }
                responder.respond(PromptResponse::new(StopReason::EndTurn))
            },
            agent_client_protocol::on_receive_request!(),
        )
}

/// An agent that answers `initialize` with the exact payload captured from
/// `claude-agent-acp` once the client advertised `auth.terminal` — replayed
/// as wire JSON rather than rebuilt from typed constructors, so the test
/// exercises the same deserialization the live adapter goes through.
fn login_advertising_agent() -> impl ConnectTo<Client> + 'static {
    use agent_client_protocol::schema::v1::{InitializeResponse, NewSessionResponse};
    Agent
        .builder()
        .on_receive_request(
            async |_req: InitializeRequest, responder, _conn| {
                let response: InitializeResponse = serde_json::from_value(serde_json::json!({
                    "protocolVersion": 1,
                    "agentCapabilities": {},
                    "authMethods": [
                        {
                            "id": "claude-ai-login",
                            "name": "Claude Subscription",
                            "description": "Use Claude subscription ",
                            "type": "terminal",
                            "args": ["--cli", "auth", "login", "--claudeai"],
                            "_meta": {"terminal-auth": {
                                "command": "/opt/node/bin/node",
                                "args": ["/cache/claude-agent-acp", "--cli", "auth",
                                         "login", "--claudeai"],
                                "label": "Claude Login"
                            }}
                        },
                        {
                            "id": "console-login",
                            "name": "Anthropic Console",
                            "description": "Use Anthropic Console (API usage billing)",
                            "type": "terminal",
                            "args": ["--cli", "auth", "login", "--console"]
                        }
                    ]
                }))
                .expect("the captured initialize payload parses");
                responder.respond(response)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |_req: NewSessionRequest, responder, _conn| {
                responder.respond(NewSessionResponse::new("sess-login-methods"))
            },
            agent_client_protocol::on_receive_request!(),
        )
}

/// An agent reporting the exact `agentInfo` captured from a live
/// claude-agent-acp `initialize`. Which dialect a session speaks is decided
/// by the program on the wire, so that program has to reach the host.
fn self_identifying_agent() -> impl ConnectTo<Client> + 'static {
    use agent_client_protocol::schema::v1::{InitializeResponse, NewSessionResponse};
    Agent
        .builder()
        .on_receive_request(
            async |_req: InitializeRequest, responder, _conn| {
                let response: InitializeResponse = serde_json::from_value(serde_json::json!({
                    "protocolVersion": 1,
                    "agentCapabilities": {},
                    "agentInfo": {
                        "name": "@agentclientprotocol/claude-agent-acp",
                        "title": "Claude Agent",
                        "version": "0.70.0"
                    }
                }))
                .expect("the captured initialize payload parses");
                responder.respond(response)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |_req: NewSessionRequest, responder, _conn| {
                responder.respond(NewSessionResponse::new("sess-agent-info"))
            },
            agent_client_protocol::on_receive_request!(),
        )
}
