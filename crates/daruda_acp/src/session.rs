//! Long-lived multi-turn ACP connection.
//!
//! Where [`crate::connection`] is the one-shot spike, this module keeps the
//! session alive across many prompts. It spawns the protocol connection as a
//! smol task and exposes a handle the host (`app`) drives by enqueuing
//! commands; the connection's `main_fn` owns the single live `connection`
//! object and is the only place that issues `send_request` /
//! `send_notification`. Protocol traffic flows back to the host as
//! [`AcpEvent`]s on an unbounded channel.
//!
//! GPUI-free: this crate never imports GPUI. The host translates events into
//! its render model (`Vec<ChatItem>`) via [`crate::mapping`].
//!
//! ## Communication shape
//!
//! ```text
//!   host (app)                       connection task (main_fn)
//!   ──────────                       ─────────────────────────
//!   send_prompt ─┐                    ┌─ select loop ─┐
//!   cancel ──────┼─ Command channel ─▶│  drains cmds  │── send_request ─▶ agent
//!                ┘                     │  awaits turn  │── send_notification ─▶ agent
//!                                      └───────────────┘
//!                                            │
//!   AcpEvent rx  ◀────────── event channel ──┘  (Connected / Update /
//!                                                 PermissionRequested /
//!                                                 TurnEnded / Error)
//!
//!   respond_permission ── parked oneshot map ──▶ on_receive_request handler
//! ```
//!
//! The permission handler is a separate closure from `main_fn`, so it cannot
//! reach the command channel's response path directly. It instead parks on a
//! `oneshot` whose sender lives in a shared map keyed by a request id; the host
//! resolves it through [`AcpSessionHandle::respond_permission`]. Awaiting in
//! the handler is safe because the connection keeps pumping concurrently.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock, InitializeRequest, NewSessionRequest, PromptRequest,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionId, SessionNotification, SessionUpdate, TextContent,
};
use agent_client_protocol::{AcpAgent, Agent, ConnectionTo};
use futures::FutureExt;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::channel::oneshot;

use crate::connection::{AcpClientError, AdapterCommand};

/// Map of in-flight permission requests awaiting a host decision: request id →
/// the oneshot sender that unparks the connection's `on_receive_request`
/// handler once the host calls [`AcpSessionHandle::respond_permission`].
type PermissionParks = Arc<Mutex<HashMap<u64, oneshot::Sender<PermissionDecision>>>>;

/// The host's decision on a permission request, in this crate's own vocabulary
/// so the host never touches protocol types. `option_id` is the choice the
/// host picked from the request's `options` (see [`crate::model::PermissionChoice`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Approve the tool call, selecting the given option.
    Allow { option_id: String },
    /// Reject the tool call, selecting the given (reject-kind) option.
    Reject { option_id: String },
    /// The turn was cancelled before the user decided.
    Cancelled,
}

impl PermissionDecision {
    /// Convert into the protocol response sent back to the agent. `Allow` and
    /// `Reject` both map to `Selected` — the agent enforces the option's
    /// semantics; the distinction is only host-side intent.
    fn into_response(self) -> RequestPermissionResponse {
        let outcome = match self {
            PermissionDecision::Allow { option_id } | PermissionDecision::Reject { option_id } => {
                RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id))
            }
            PermissionDecision::Cancelled => RequestPermissionOutcome::Cancelled,
        };
        RequestPermissionResponse::new(outcome)
    }
}

/// An event emitted by the live connection for the host to consume.
#[derive(Debug)]
pub enum AcpEvent {
    /// `initialize` + `session/new` succeeded; the session is ready for prompts.
    Connected,
    /// A `session/update` notification arrived. The host folds it into its
    /// chat model via [`crate::mapping::apply_update`].
    Update(Box<SessionUpdate>),
    /// The agent requested tool permission. The host renders the request, then
    /// calls [`AcpSessionHandle::respond_permission`] with the matching `id`.
    PermissionRequested {
        id: u64,
        request: Box<RequestPermissionRequest>,
    },
    /// A `session/prompt` turn completed; carries the protocol stop reason.
    TurnEnded { stop_reason: String },
    /// A connection or protocol failure. Terminal: the connection task is
    /// ending (or has ended) when this is emitted.
    Error(String),
}

/// A command the host enqueues for the connection task to execute. Internal:
/// the handle's public methods build these.
enum Command {
    /// Send a `session/prompt` with this user text.
    Prompt(String),
    /// Send a `session/cancel` notification for the active turn.
    Cancel,
}

/// Host-side handle to a live ACP session. Cloning is intentionally not derived
/// — the host holds one handle; dropping it (and thus closing the command
/// channel) tells the connection task to shut down.
pub struct AcpSessionHandle {
    commands: UnboundedSender<Command>,
    permission_parks: PermissionParks,
}

impl AcpSessionHandle {
    /// Queue a user prompt for the next turn. Returns immediately; the turn's
    /// completion surfaces as [`AcpEvent::TurnEnded`].
    ///
    /// A send failure means the connection task has already ended; the error
    /// is dropped because the host learns of termination via the event stream
    /// (an [`AcpEvent::Error`] or end-of-stream).
    pub fn send_prompt(&self, text: String) {
        let _ = self.commands.unbounded_send(Command::Prompt(text));
    }

    /// Request cancellation of the active turn via `session/cancel`. The agent
    /// still finishes flushing updates and returns a `Cancelled` stop reason,
    /// surfaced as a normal [`AcpEvent::TurnEnded`].
    pub fn cancel(&self) {
        let _ = self.commands.unbounded_send(Command::Cancel);
    }

    /// Resolve a parked permission request the host received as
    /// [`AcpEvent::PermissionRequested`]. Unparks the connection's handler,
    /// which then responds to the agent. A no-op if the id is unknown (already
    /// resolved, or the turn was cancelled).
    pub fn respond_permission(&self, id: u64, decision: PermissionDecision) {
        let sender = self
            .permission_parks
            .lock()
            .expect("permission parks mutex poisoned")
            .remove(&id);
        if let Some(sender) = sender {
            // The receiver is gone only if the handler already unparked (e.g.
            // the connection dropped); nothing to do in that case.
            let _ = sender.send(decision);
        }
    }
}

/// Open a long-lived ACP session against `command`, rooted at `cwd`.
///
/// Spawns the protocol connection as a detached smol task and returns a handle
/// plus the event receiver. The task runs until the handle is dropped (command
/// channel closes) or the connection fails; either way the event stream then
/// reaches end-of-stream. A failure to *parse* the adapter command is reported
/// synchronously as an error here, before any task is spawned.
pub fn connect_session(
    command: AdapterCommand,
    cwd: PathBuf,
) -> Result<(AcpSessionHandle, UnboundedReceiver<AcpEvent>), AcpClientError> {
    let agent =
        AcpAgent::from_str(&command.0).map_err(|e| AcpClientError::Command(format!("{e:?}")))?;

    let (command_tx, command_rx) = unbounded::<Command>();
    let (event_tx, event_rx) = unbounded::<AcpEvent>();
    let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));

    let handle = AcpSessionHandle {
        commands: command_tx,
        permission_parks: permission_parks.clone(),
    };

    let task_event_tx = event_tx.clone();
    smol::spawn(async move {
        if let Err(err) = run_connection(
            agent,
            cwd,
            command_rx,
            task_event_tx.clone(),
            permission_parks,
        )
        .await
        {
            // Terminal failure: surface it, then let the channel drop close the
            // stream. `unbounded_send` only fails if the host stopped reading.
            let _ = task_event_tx.unbounded_send(AcpEvent::Error(format!("{err}")));
        }
    })
    .detach();

    Ok((handle, event_rx))
}

/// Drive the whole connection: handshake, session creation, then the prompt /
/// cancel select loop, until the command channel closes or the protocol fails.
async fn run_connection(
    agent: AcpAgent,
    cwd: PathBuf,
    command_rx: UnboundedReceiver<Command>,
    event_tx: UnboundedSender<AcpEvent>,
    permission_parks: PermissionParks,
) -> Result<(), AcpClientError> {
    let notif_tx = event_tx.clone();
    let perm_event_tx = event_tx.clone();
    let next_permission_id = Arc::new(AtomicU64::new(0));

    agent_client_protocol::Client
        .builder()
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                // The host owns the mapping into its chat model; forward raw.
                let _ = notif_tx.unbounded_send(AcpEvent::Update(Box::new(notification.update)));
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            {
                let permission_parks = permission_parks.clone();
                let next_permission_id = next_permission_id.clone();
                let perm_event_tx = perm_event_tx.clone();
                async move |request: RequestPermissionRequest, responder, _connection| {
                    let id = next_permission_id.fetch_add(1, Ordering::Relaxed);
                    let (decision_tx, decision_rx) = oneshot::channel::<PermissionDecision>();
                    permission_parks
                        .lock()
                        .expect("permission parks mutex poisoned")
                        .insert(id, decision_tx);

                    let _ = perm_event_tx.unbounded_send(AcpEvent::PermissionRequested {
                        id,
                        request: Box::new(request),
                    });

                    // Park until the host decides. The connection keeps pumping
                    // concurrently, so this await does not block other traffic.
                    // If the sender is dropped (host went away / id reaped on
                    // shutdown), default to Cancelled — deny by absence.
                    let decision = decision_rx.await.unwrap_or(PermissionDecision::Cancelled);
                    responder.respond(decision.into_response())
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, move |connection: ConnectionTo<Agent>| async move {
            connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;

            let new_session = connection
                .send_request(NewSessionRequest::new(cwd))
                .block_task()
                .await?;
            let session_id = new_session.session_id;
            let _ = event_tx.unbounded_send(AcpEvent::Connected);

            prompt_loop(&connection, session_id, command_rx, &event_tx).await?;
            Ok(())
        })
        .await
        .map_err(|e| AcpClientError::Protocol(format!("{e:?}")))?;

    Ok(())
}

/// The multi-turn pump. Runs prompts strictly one turn at a time, draining a
/// stash of prompts that arrived mid-turn before pulling new commands, and
/// ending when the command channel closes (handle dropped).
///
/// Turns are serialized because a session is single-threaded: a second
/// `session/prompt` cannot be issued until the first turn's stop reason
/// returns. The stash preserves arrival order without losing any prompt.
async fn prompt_loop(
    connection: &ConnectionTo<Agent>,
    session_id: SessionId,
    mut command_rx: UnboundedReceiver<Command>,
    event_tx: &UnboundedSender<AcpEvent>,
) -> Result<(), agent_client_protocol::Error> {
    let mut stash: VecDeque<String> = VecDeque::new();
    loop {
        // Prefer a prompt queued during the previous turn; otherwise block on
        // the channel for the next command.
        let text = match stash.pop_front() {
            Some(text) => text,
            None => match command_rx.next().await {
                Some(Command::Prompt(text)) => text,
                // A cancel with no active turn has nothing to cancel; ignore.
                Some(Command::Cancel) => continue,
                None => return Ok(()),
            },
        };
        let dropped = run_turn(
            connection,
            &session_id,
            text,
            &mut command_rx,
            &mut stash,
            event_tx,
        )
        .await?;
        if dropped {
            // Handle was dropped mid-turn: no more commands will arrive.
            return Ok(());
        }
    }
}

/// Send one `session/prompt` and await its stop reason while servicing commands
/// that arrive mid-turn: a `Cancel` is forwarded as `session/cancel`; a queued
/// `Prompt` is stashed for the outer loop to run next (never dropped). Returns
/// `true` if the command channel closed (handle dropped) during the turn.
async fn run_turn(
    connection: &ConnectionTo<Agent>,
    session_id: &SessionId,
    text: String,
    command_rx: &mut UnboundedReceiver<Command>,
    stash: &mut VecDeque<String>,
    event_tx: &UnboundedSender<AcpEvent>,
) -> Result<bool, agent_client_protocol::Error> {
    let response = connection
        .send_request(PromptRequest::new(
            session_id.clone(),
            vec![ContentBlock::Text(TextContent::new(text))],
        ))
        .block_task()
        .fuse();
    // `block_task()`'s future is `!Unpin`; `select!` requires `Unpin`.
    futures::pin_mut!(response);

    let mut handle_dropped = false;
    let stop_reason = loop {
        futures::select! {
            resp = response => break resp?.stop_reason,
            command = command_rx.next() => match command {
                Some(Command::Cancel) => {
                    connection.send_notification(CancelNotification::new(session_id.clone()))?;
                    // Keep awaiting: the agent still returns a (Cancelled) stop
                    // reason and may flush final updates first.
                }
                Some(Command::Prompt(queued)) => {
                    // Sessions are single-turn; run it after this turn ends.
                    stash.push_back(queued);
                }
                None => {
                    // Handle dropped mid-turn: cancel and let the turn wind
                    // down. `UnboundedReceiver` is a `FusedStream`, so once it
                    // reports `None` `select!` stops polling it — no spin — and
                    // the loop now waits only on `response`.
                    connection.send_notification(CancelNotification::new(session_id.clone()))?;
                    handle_dropped = true;
                }
            },
        }
    };

    let _ = event_tx.unbounded_send(AcpEvent::TurnEnded {
        stop_reason: format!("{stop_reason:?}"),
    });
    Ok(handle_dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::PermissionOptionId;

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
}
