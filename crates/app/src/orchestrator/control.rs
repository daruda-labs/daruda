//! Answering one MCP frame.
//!
//! The seam between the protocol and the app: `tools/call` is intercepted here
//! because it needs app state, and everything else falls through to
//! `mcp::protocol`, which needs none.
//!
//! Order for a call is **name → arguments → guards → gate → execute**, and it
//! matters. A tool the agent may not use at all, or arguments that cannot
//! become a command, are refused before any window is touched; a spent budget
//! or a pane it may not address is refused before the user's phone buzzes.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{App, AppContext as _};

use crate::control::approval::ApprovalId;
use crate::control::exec::{self, Dispatch};
use crate::control::guards;
use crate::control::mcp::convert::{self, Command};
use crate::control::mcp::protocol::{self, Session};

/// The handshake state of whichever connection is currently served, plus the
/// calls it is still waiting on.
///
/// Keyed by connection so a reconnect starts over. One at a time, so one slot.
#[derive(Default)]
pub(crate) struct ConnectionSession {
    connection: u64,
    session: Session,
    /// Calls whose reply has not been sent, by the JSON text of their id — the
    /// only handle a `notifications/cancelled` gives us.
    ///
    /// Bounded by pruning, not by a cap: an entry is worth keeping only while
    /// its card is still waiting, and once it is not there is nothing left to
    /// withdraw.
    outstanding: HashMap<String, Outstanding>,
}

/// A `tools/call` waiting on the user.
struct Outstanding {
    approval: ApprovalId,
    /// Set when the caller withdraws the call. The task that would send the
    /// reply reads it instead of answering a request the client has freed.
    withdrawn: Arc<AtomicBool>,
}

impl ConnectionSession {
    /// A session already past the handshake, for a test that is about what
    /// happens *after* it.
    #[cfg(test)]
    fn handshaked_for_test(connection: u64) -> Self {
        Self {
            connection,
            session: Session::ready_for_test(),
            outstanding: HashMap::new(),
        }
    }

    fn for_connection(&mut self, connection: u64) -> &mut Session {
        if self.connection != connection {
            self.connection = connection;
            self.session = Session::default();
            // A new connection cannot cancel the previous one's calls, and
            // whatever they were waiting on is answered by its own card.
            self.outstanding.clear();
        }
        &mut self.session
    }

    /// Remember a call that has not been answered yet, and forget the ones
    /// whose card has since been decided.
    fn track(&mut self, request: String, approval: ApprovalId, cx: &App) -> Arc<AtomicBool> {
        self.outstanding
            .retain(|_, o| crate::control::approval::is_waiting(o.approval, cx));
        let withdrawn = Arc::new(AtomicBool::new(false));
        self.outstanding.insert(
            request,
            Outstanding {
                approval,
                withdrawn: withdrawn.clone(),
            },
        );
        withdrawn
    }

    /// Take back the call `request` names: the card comes down, the work never
    /// starts, and no reply is sent.
    ///
    /// A cancellation for a call already answered — or one that was never
    /// gated — finds nothing, which is the right outcome either way: the
    /// effect it wanted to prevent has already happened or was never pending.
    fn withdraw(&mut self, request: &str, cx: &mut App) {
        let Some(entry) = self.outstanding.remove(request) else {
            return;
        };
        // Before withdrawing: the flag is what suppresses the reply, and
        // settling the card is what wakes the task that would send it.
        entry.withdrawn.store(true, Ordering::SeqCst);
        crate::control::approval::withdraw(entry.approval, cx);
    }
}
use crate::control::mcp::socket::Inbound;
use crate::control::mcp::tools::{Gate, ToolId, ToolTable};
use crate::control::result::{ControlError, ControlOutcome};
use crate::control::spec::ResolvedCommand;

/// What answering a frame produced.
enum Answer {
    /// Send this back now.
    Now(String),
    /// A notification: no reply, ever.
    Silence,
    /// The tool is still waiting on the user. Whoever awaits `outcome` sends
    /// the reply — unless the call is withdrawn first.
    Later {
        id: serde_json::Value,
        outcome: smol::channel::Receiver<ControlOutcome>,
        approval: ApprovalId,
    },
}

/// Handle one frame from the socket, replying on its channel.
///
/// Spawns only for a call that has to wait, so a `ping` never queues behind a
/// pending approval.
pub(crate) fn answer(message: Inbound, session: &mut ConnectionSession, cx: &mut App) {
    let Inbound {
        frame,
        reply,
        connection,
    } = message;
    // A new connection is a new session. The socket serves them one at a time
    // but *sequentially*, so without this a second shim would inherit the
    // first's completed handshake and could call a tool having never sent
    // `initialize` — the one admission gate, off from connection two on.
    session.for_connection(connection);
    // A cancellation is the one notification that means something here: it
    // names a call the app may still be able to stop.
    if let Some(request) = protocol::cancellation_target(&frame) {
        session.withdraw(&request, cx);
        return;
    }
    match answer_frame(&frame, &mut session.session, cx) {
        Answer::Silence => {}
        Answer::Now(text) => {
            // Bounded at one and never contended: the socket task awaits
            // exactly this send.
            if reply.try_send(text).is_err() {
                log_unanswered(connection);
            }
        }
        Answer::Later {
            id,
            outcome,
            approval,
        } => {
            let withdrawn = session.track(protocol::request_key(&id), approval, cx);
            cx.background_spawn(async move {
                // A closed channel means the task that would have answered
                // is gone (the app is shutting down) — the target, not the
                // orchestrator, is what became unreachable.
                let result = outcome
                    .recv()
                    .await
                    .unwrap_or(Err(ControlError::TargetGone));
                // The caller took the call back, so per the cancellation spec
                // it gets no response — it has freed the id and a reply would
                // be answering a question nobody asked.
                if withdrawn.load(Ordering::SeqCst) {
                    return;
                }
                if reply
                    .send(protocol::result_frame(id, convert::to_tool_result(&result)))
                    .await
                    .is_err()
                {
                    log_unanswered(connection);
                }
            })
            .detach();
        }
    }
}

/// A reply that could not be handed back to the socket.
///
/// Only reachable once the connection is gone, so there is nobody left to tell
/// — which is exactly why it is worth a line: "the orchestrator hung" is
/// otherwise indistinguishable from a tool that never answered.
fn log_unanswered(connection: u64) {
    daruda_store::observability::log_writer::LogWriter::log(
        daruda_store::observability::error_report::ErrorReport::new(
            "Control reply had nowhere to go: the connection closed first",
        )
        .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
        .with_context("connection", connection.to_string())
        .at(file!(), line!())
        .dedup("control.reply.undeliverable")
        .build(),
    );
}

fn answer_frame(frame: &str, session: &mut Session, cx: &mut App) -> Answer {
    let tools = ToolTable::all();
    // Only `tools/call` needs the app; peeking for it keeps the rest of the
    // protocol in the one place that owns it.
    let Some(call) = ToolCall::parse(frame) else {
        return match protocol::handle(frame, session, &tools) {
            Some(text) => Answer::Now(text),
            None => Answer::Silence,
        };
    };
    // The same admission question `handle` asks, asked here because the host
    // answers this method itself.
    if let Some(refusal) = session.refuse_if_premature("tools/call", &call.id) {
        return Answer::Now(refusal);
    }

    // A call that named no tool is answered as the missing argument it is,
    // not as a tool called "" that does not exist.
    let name = match call.name {
        Ok(name) => name,
        Err(e) => {
            return Answer::Now(protocol::result_frame(
                call.id,
                convert::convert_error_result(&e),
            ));
        }
    };
    let Some(id) = tools.lookup(&name) else {
        return Answer::Now(protocol::result_frame(
            call.id,
            convert::unknown_tool_result(&name),
        ));
    };
    let command = match convert::to_command(id, &call.arguments) {
        Ok(command) => command,
        Err(e) => {
            return Answer::Now(protocol::result_frame(
                call.id,
                convert::convert_error_result(&e),
            ));
        }
    };

    match command {
        Command::Immediate(resolved) => {
            let outcome = match guard_immediate(id, &resolved, cx) {
                Ok(()) => exec::run(resolved, cx),
                Err(refusal) => Err(refusal),
            };
            Answer::Now(protocol::result_frame(
                call.id,
                convert::to_tool_result(&outcome),
            ))
        }
        Command::Gated(gated) => match exec::run_gated(gated, cx) {
            Dispatch::Ready(outcome) => Answer::Now(protocol::result_frame(
                call.id,
                convert::to_tool_result(&outcome),
            )),
            Dispatch::Deferred { outcome, approval } => Answer::Later {
                id: call.id,
                outcome,
                approval,
            },
        },
    }
}

/// The two guards a non-creating tool has to clear.
///
/// The budget guard belongs to the gated path, which spends it; these two are
/// about the *target*, so they apply wherever a tool names one.
fn guard_immediate(
    id: ToolId,
    resolved: &ResolvedCommand,
    cx: &mut App,
) -> Result<(), ControlError> {
    // Belt to the braces of
    // `convert::tests::the_declared_gate_and_the_converted_command_agree`,
    // which is what actually binds the gate to the command shape — this only
    // catches it at the call site in a debug build.
    debug_assert_eq!(
        ToolTable::all().gate(id),
        Gate::Open,
        "a gated tool must not reach the immediate path"
    );
    match resolved {
        ResolvedCommand::Say { target, .. } => {
            guards::guard_self_target(*target, cx)?;
            guards::guard_queue_depth(*target, cx)
        }
        // A stop is how a runaway is *ended*, so it is never refused for
        // addressing the orchestrator; a listing names nothing.
        ResolvedCommand::List
        | ResolvedCommand::Brief
        | ResolvedCommand::LaneList
        | ResolvedCommand::Stop { .. }
        | ResolvedCommand::Flow(_)
        | ResolvedCommand::Ask { .. } => Ok(()),
    }
}

/// A parsed `tools/call`. `None` for any other method, which the protocol
/// layer handles.
struct ToolCall {
    id: serde_json::Value,
    /// `Err` when the call named no tool: a distinct answer from "no such
    /// tool", because the model fixes the two differently.
    name: Result<String, convert::ConvertError>,
    arguments: serde_json::Value,
}

impl ToolCall {
    fn parse(frame: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(frame).ok()?;
        let members = value.as_object()?;
        if members.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
            return None;
        }
        // A call without an id is a notification, which cannot be answered —
        // and a tool call nobody hears the result of is not one worth running.
        // The rule itself lives in `protocol`, so both sides classify alike.
        let id = protocol::request_id(members)?;
        Some(Self {
            id,
            name: match value.pointer("/params/name") {
                Some(name) => name
                    .as_str()
                    .map(str::to_owned)
                    .ok_or(convert::ConvertError::BadArgument { name: "name" }),
                None => Err(convert::ConvertError::MissingArgument { name: "name" }),
            },
            arguments: value
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: i64, name: &str, arguments: serde_json::Value) -> String {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        })
        .to_string()
    }

    use gpui::TestAppContext;

    /// Drive one frame the way the socket task does, through a session that
    /// has already handshaked.
    fn answered(frame: &str, cx: &mut TestAppContext) -> serde_json::Value {
        let text = answer_on(frame, ConnectionSession::handshaked_for_test(1), 1, cx)
            .try_recv()
            .expect("answered now");
        serde_json::from_str(&text).expect("json")
    }

    /// Drive one frame the way the socket task does, returning the reply
    /// channel so a deferred answer can be awaited.
    fn answer_on(
        frame: &str,
        mut session: ConnectionSession,
        connection: u64,
        cx: &mut TestAppContext,
    ) -> smol::channel::Receiver<String> {
        let (tx, rx) = smol::channel::bounded(1);
        cx.update(|cx| {
            answer(
                Inbound {
                    frame: frame.to_owned(),
                    reply: tx,
                    connection,
                },
                &mut session,
                cx,
            );
        });
        rx
    }

    /// The payload the model reads, out of the one text block.
    /// A configuration and a bridge that can actually deliver a card.
    fn reachable_phone(cx: &mut App) {
        use gpui::BorrowAppContext as _;

        crate::settings_store::SettingsStore::init(cx);
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            let mut cfg = (*store.user()).clone();
            cfg.telegram.enabled = true;
            cfg.telegram.authorized_chat_id = Some(42);
            store.set_user_for_testing(cfg);
        });
        let _outbound = crate::telegram::global::install_for_test(true, Some(42), cx);
    }

    fn cancellation(request_id: i64) -> String {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": { "requestId": request_id, "reason": "the turn was stopped" },
        })
        .to_string()
    }

    /// One frame into a session the caller keeps, for a test that spans two.
    fn deliver(
        frame: &str,
        session: &mut ConnectionSession,
        cx: &mut TestAppContext,
    ) -> smol::channel::Receiver<String> {
        let (tx, rx) = smol::channel::bounded(1);
        cx.update(|cx| {
            answer(
                Inbound {
                    frame: frame.to_owned(),
                    reply: tx,
                    connection: session.connection.max(1),
                },
                session,
                cx,
            );
        });
        rx
    }

    fn payload(reply: &serde_json::Value) -> serde_json::Value {
        let text = reply["result"]["content"][0]["text"]
            .as_str()
            .expect("text block");
        serde_json::from_str(text).expect("json payload")
    }

    #[gpui::test]
    async fn a_tool_call_reaches_the_control_core(cx: &mut TestAppContext) {
        let _fixture = crate::test_support::workspace_with_agent_chat(cx);
        let reply = answered(&call(1, "daruda_chat_list", serde_json::json!({})), cx);
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["isError"], false);
        let inner = payload(&reply);
        assert_eq!(inner["kind"], "listing");
        assert!(
            !inner["windows"].as_array().expect("windows").is_empty(),
            "the fixture's chat is listed: {inner}"
        );
    }

    /// A read of every worktree, including ones with no chat — the listing
    /// `/list` answers with cannot name those.
    #[gpui::test]
    async fn lane_list_answers_from_the_same_frame_path(cx: &mut TestAppContext) {
        let _fixture = crate::test_support::workspace_with_agent_chat(cx);
        let reply = answered(&call(2, "daruda_worktree_list", serde_json::json!({})), cx);
        assert_eq!(reply["result"]["isError"], false);
        let inner = payload(&reply);
        assert_eq!(inner["kind"], "lane_listing");
        let lanes = inner["lanes"].as_array().expect("lanes");
        assert!(!lanes.is_empty());
        assert!(
            lanes[0]["target"]["workspace"].is_string(),
            "a handle carries its window: {inner}"
        );
    }

    /// The budget is not a decision the user gets to make, so it is answered
    /// without a card ever reaching their phone.
    #[gpui::test]
    async fn a_gated_tool_refuses_when_the_budget_is_spent(cx: &mut TestAppContext) {
        let fixture = crate::test_support::workspace_with_agent_chat(cx);
        let (workspace, project) = fixture
            .workspace
            .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane().project));
        cx.update(|cx| {
            crate::settings_store::SettingsStore::init(cx);
            crate::telegram::global::install_for_test(false, None, cx);
            for _ in 0..guards::AGENT_LANE_BUDGET {
                guards::note_agent_created_lane(cx);
            }
        });

        let reply = answered(
            &call(
                3,
                "daruda_worktree_create",
                serde_json::json!({ "workspace": workspace, "project": project, "name": "x" }),
            ),
            cx,
        );
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(payload(&reply)["code"], "agent_limit_reached");
        cx.update(|cx| {
            assert_eq!(
                crate::control::approval::waiting_count_for_test(cx),
                0,
                "refused before the card"
            );
        });
    }

    /// The deferred path, end to end: a gated call whose answer arrives later
    /// still comes back on the *same* JSON-RPC id, with the outcome the gate
    /// produced. Nothing else exercises `Answer::Later`'s success branch —
    /// `answered()` asserts the reply is already there, which is
    /// `Answer::Now` by construction.
    ///
    /// The card is undeliverable (no bridge configured), which is the one way
    /// to make a *real* gated call settle inside a test without a phone.
    #[gpui::test]
    async fn a_deferred_answer_comes_back_on_the_same_id(cx: &mut TestAppContext) {
        let fixture = crate::test_support::workspace_with_agent_chat(cx);
        let (workspace, project) = fixture
            .workspace
            .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane().project));
        cx.update(|cx| {
            crate::settings_store::SettingsStore::init(cx);
            crate::telegram::global::install_for_test(false, None, cx);
        });

        let mut session = ConnectionSession::handshaked_for_test(1);
        let reply = deliver(
            &call(
                4242,
                "daruda_worktree_create",
                serde_json::json!({ "workspace": workspace, "project": project, "name": "x" }),
            ),
            &mut session,
            cx,
        );
        assert!(
            reply.try_recv().is_err(),
            "the gate answers later, not inline"
        );

        cx.run_until_parked();
        let text = reply.try_recv().expect("the deferred reply arrives");
        let answer: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(
            answer["id"], 4242,
            "a reply on the wrong id is a reply the client cannot match: {answer}"
        );
        assert_eq!(answer["result"]["isError"], true);
        assert_eq!(payload(&answer)["code"], "approval_unavailable");
    }

    /// A `notifications/cancelled` for a call still waiting on a card: the
    /// question comes down, the work never starts, and the client that freed
    /// the request id gets no reply for it.
    #[gpui::test]
    async fn cancelling_a_waiting_call_withdraws_it_unanswered(cx: &mut TestAppContext) {
        let fixture = crate::test_support::workspace_with_agent_chat(cx);
        let (workspace, project) = fixture
            .workspace
            .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane().project));
        // Deliverable, so the card is really waiting rather than settling as
        // undeliverable before the cancellation arrives. Both halves matter:
        // the *config* is what `send_approval_card` reads, and the bridge
        // global is what holds the card's tokens.
        cx.update(reachable_phone);

        let mut session = ConnectionSession::handshaked_for_test(1);
        let reply = deliver(
            &call(
                7,
                "daruda_worktree_create",
                serde_json::json!({ "workspace": workspace, "project": project, "name": "x" }),
            ),
            &mut session,
            cx,
        );
        cx.update(|cx| {
            assert_eq!(
                crate::control::approval::waiting_count_for_test(cx),
                1,
                "the call is waiting on the phone"
            );
        });
        assert!(reply.try_recv().is_err(), "and nothing is answered yet");

        let _ = deliver(&cancellation(7), &mut session, cx);
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(
                crate::control::approval::waiting_count_for_test(cx),
                0,
                "the withdrawn question is off the phone"
            );
        });
        assert!(
            reply.try_recv().is_err(),
            "a withdrawn call is not answered — the client has freed its id"
        );
    }

    /// A cancellation naming a call that was never pending must be a no-op,
    /// not a panic and not a withdrawal of whatever else is waiting.
    #[gpui::test]
    async fn cancelling_something_unknown_leaves_the_waiting_call_alone(cx: &mut TestAppContext) {
        let fixture = crate::test_support::workspace_with_agent_chat(cx);
        let (workspace, project) = fixture
            .workspace
            .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane().project));
        cx.update(reachable_phone);

        let mut session = ConnectionSession::handshaked_for_test(1);
        let _reply = deliver(
            &call(
                7,
                "daruda_worktree_create",
                serde_json::json!({ "workspace": workspace, "project": project, "name": "x" }),
            ),
            &mut session,
            cx,
        );
        // A different id, and the string spelling of the same one: neither
        // names the call that is waiting.
        let _ = deliver(&cancellation(8), &mut session, cx);
        let _ = deliver(
            &serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": { "requestId": "7" },
            })
            .to_string(),
            &mut session,
            cx,
        );
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(
                crate::control::approval::waiting_count_for_test(cx),
                1,
                "the waiting call is untouched"
            );
        });
    }

    /// MCP wants an unknown tool reported as a *tool* error so the model can
    /// recover by listing again, not as a transport failure it may never see.
    #[gpui::test]
    async fn an_unknown_tool_name_is_an_error_result_not_a_protocol_error(cx: &mut TestAppContext) {
        let _fixture = crate::test_support::workspace_with_agent_chat(cx);
        let reply = answered(&call(4, "daruda_nope", serde_json::json!({})), cx);
        assert!(reply.get("error").is_none(), "not a protocol error");
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(payload(&reply)["code"], "unknown_tool");
        assert_eq!(payload(&reply)["tool"], "daruda_nope");
    }

    /// Bad arguments name the argument, so the model can fix them.
    #[gpui::test]
    async fn bad_arguments_are_named_in_the_tool_error(cx: &mut TestAppContext) {
        let _fixture = crate::test_support::workspace_with_agent_chat(cx);
        let reply = answered(&call(5, "daruda_chat_send", serde_json::json!({})), cx);
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(payload(&reply)["code"], "missing_argument");
        assert_eq!(payload(&reply)["argument"], "target");
    }

    /// The orchestrator prompting itself is the one pure software loop left,
    /// and it is refused at the frame boundary.
    #[gpui::test]
    async fn a_self_addressed_prompt_is_refused(cx: &mut TestAppContext) {
        let own = crate::test_support::register_test_orchestrator(cx);
        let reply = answered(
            &call(
                6,
                "daruda_chat_send",
                serde_json::json!({ "target": own, "text": "go" }),
            ),
            cx,
        );
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(payload(&reply)["code"], "self_target_refused");
    }

    /// A stop is how a runaway is *ended*, so it must not be refused for
    /// naming the orchestrator.
    #[gpui::test]
    async fn stopping_the_orchestrator_is_allowed(cx: &mut TestAppContext) {
        let own = crate::test_support::register_test_orchestrator(cx);
        let reply = answered(
            &call(7, "daruda_chat_stop", serde_json::json!({ "target": own })),
            cx,
        );
        assert_eq!(
            reply["result"]["isError"], false,
            "a stop is the way out of a loop: {reply}"
        );
    }

    /// A frame that is not a call still gets its protocol answer through this
    /// same entry point — the host must not swallow `tools/list`.
    #[gpui::test]
    async fn a_non_call_frame_falls_through_to_the_protocol_layer(cx: &mut TestAppContext) {
        let reply = answered(r#"{"jsonrpc":"2.0","id":8,"method":"tools/list"}"#, cx);
        assert_eq!(reply["id"], 8);
        assert_eq!(reply["result"]["tools"].as_array().expect("tools").len(), 9);
    }

    /// A premature call is refused, and by the same predicate `handle` uses.
    #[gpui::test]
    async fn a_call_before_the_handshake_is_refused(cx: &mut TestAppContext) {
        let text = answer_on(
            &call(9, "daruda_status", serde_json::json!({})),
            ConnectionSession::default(),
            1,
            cx,
        )
        .try_recv()
        .expect("answered now");
        let reply: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(reply["error"]["code"], protocol::NOT_INITIALIZED);
    }

    /// The socket serves connections one at a time but *sequentially*, so a
    /// reconnect must start over — otherwise shim two inherits shim one's
    /// handshake and can call a tool having never sent `initialize`.
    #[gpui::test]
    async fn a_second_connection_does_not_inherit_the_first_handshake(cx: &mut TestAppContext) {
        let mut session = ConnectionSession::handshaked_for_test(1);
        let text = answer_on(
            &call(1, "daruda_status", serde_json::json!({})),
            std::mem::take(&mut session),
            2,
            cx,
        )
        .try_recv()
        .expect("answered now");
        let reply: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(
            reply["error"]["code"],
            protocol::NOT_INITIALIZED,
            "connection two starts over: {reply}"
        );
    }

    #[test]
    fn a_tool_call_is_recognised_and_its_parts_extracted() {
        let parsed = ToolCall::parse(&call(1, "daruda_chat_list", serde_json::json!({"a": 1})))
            .expect("a tools/call");
        assert_eq!(parsed.id, 1);
        assert_eq!(parsed.name.as_deref(), Ok("daruda_chat_list"));
        assert_eq!(parsed.arguments["a"], 1);
    }

    /// Missing `params` is not a parse failure — it is a call with no
    /// arguments, which four of the nine tools legitimately are. The *name*
    /// is a different matter: there is no tool without one.
    #[test]
    fn a_call_without_params_reads_as_no_arguments_and_no_name() {
        let frame = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call"}"#;
        let parsed = ToolCall::parse(frame).expect("a tools/call");
        assert_eq!(
            parsed.name,
            Err(convert::ConvertError::MissingArgument { name: "name" })
        );
        assert_eq!(parsed.arguments, serde_json::json!({}));
    }

    /// A call that named no tool, or named one with something that is not a
    /// string, must not be reported as a tool called "" that does not exist —
    /// the model would go looking for a name it never sent.
    #[gpui::test]
    async fn a_call_with_no_usable_name_says_so(cx: &mut TestAppContext) {
        let _fixture = crate::test_support::workspace_with_agent_chat(cx);
        for (frame, code, expected) in [
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#,
                "missing_argument",
                "name",
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":7}}"#,
                "bad_argument",
                "name",
            ),
        ] {
            let reply = answered(frame, cx);
            assert_eq!(reply["result"]["isError"], true, "{frame}");
            assert_eq!(payload(&reply)["code"], code, "{frame}");
            assert_eq!(payload(&reply)["argument"], expected, "{frame}");
        }
    }

    #[test]
    fn another_method_is_left_to_the_protocol_layer() {
        for frame in [
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            "{not json",
            "[]",
        ] {
            assert!(ToolCall::parse(frame).is_none(), "{frame}");
        }
    }

    /// A call with no id cannot be answered, so it is not run: a tool whose
    /// result nobody hears is all effect and no report.
    #[test]
    fn a_call_with_no_id_is_not_treated_as_a_call() {
        for frame in [
            r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"daruda_status"}}"#,
            r#"{"jsonrpc":"2.0","id":null,"method":"tools/call","params":{"name":"daruda_status"}}"#,
        ] {
            assert!(ToolCall::parse(frame).is_none(), "{frame}");
        }
    }
}
