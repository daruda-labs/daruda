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
use crate::telegram::bridge::PaneRef;

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
pub(crate) fn answer(
    message: Inbound,
    session: &mut ConnectionSession,
    protected: Option<PaneRef>,
    cx: &mut App,
) {
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
    match answer_frame(&frame, &mut session.session, protected, cx) {
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

fn answer_frame(
    frame: &str,
    session: &mut Session,
    protected: Option<PaneRef>,
    cx: &mut App,
) -> Answer {
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
            let outcome = match guard_immediate(id, &resolved, protected, cx) {
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
    protected: Option<PaneRef>,
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
            guards::guard_self_target(*target, protected)?;
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
mod tests;
