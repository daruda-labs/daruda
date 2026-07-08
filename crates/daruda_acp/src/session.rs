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
    CancelNotification, ContentBlock, InitializeRequest, LoadSessionRequest, NewSessionRequest,
    PromptRequest, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionId, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionModeRequest, StopReason, TextContent,
};
use agent_client_protocol::{AcpAgent, Agent, ConnectionTo, LineDirection};
use futures::FutureExt;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::channel::oneshot;

use crate::connection::{AcpClientError, AdapterCommand};
use crate::model::{ConfigOptionView, ModeStateView};

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

/// A change to one `SessionInfoUpdate` field. Mirrors the protocol's tri-state
/// `MaybeUndefined`: the field may be absent (untouched), explicitly cleared, or
/// set to a value — three distinct states, so a plain `Option<String>` (which
/// can't tell "untouched" from "cleared") would be lossy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfoFieldChange {
    /// The update omitted this field — leave the host's current value untouched.
    Unchanged,
    /// The update set this field to null — clear the host's value.
    Cleared,
    /// The update set this field to a concrete value.
    Set(String),
}

impl From<agent_client_protocol::schema::MaybeUndefined<String>> for InfoFieldChange {
    fn from(field: agent_client_protocol::schema::MaybeUndefined<String>) -> Self {
        use agent_client_protocol::schema::MaybeUndefined;
        match field {
            MaybeUndefined::Undefined => InfoFieldChange::Unchanged,
            MaybeUndefined::Null => InfoFieldChange::Cleared,
            MaybeUndefined::Value(v) => InfoFieldChange::Set(v),
        }
    }
}

/// An event emitted by the live connection for the host to consume.
#[derive(Debug)]
pub enum AcpEvent {
    /// `initialize` + `session/new` succeeded; the session is ready for prompts.
    /// `modes` carries the advertised session-mode state when the agent supports
    /// modes; `None` otherwise. `config_options` carries the advertised select
    /// config options (model / effort / etc.); empty when the agent advertises
    /// none.
    Connected {
        /// The live session id — from `session/new`, or the id passed to
        /// `session/load` on a resume. The host persists it so a later launch
        /// can resume the same session via [`connect_session`]'s `resume` arg.
        session_id: String,
        modes: Option<ModeStateView>,
        config_options: Vec<ConfigOptionView>,
    },
    /// The agent replaced its session config option state (the protocol carries
    /// the full option set), from either source: the reply to our
    /// `set_config_option` request, or an agent-pushed `ConfigOptionUpdate`
    /// notification (a change the agent made itself — e.g. a fast-mode toggle).
    /// Either way it is a full replacement of the host's cached options.
    ConfigOptionsChanged(Vec<ConfigOptionView>),
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
    /// `completed_normally` distinguishes a normal completion (any stop reason
    /// other than `Cancelled`) from a client-initiated cancellation, so the host
    /// need not parse the Debug-formatted `stop_reason` string.
    TurnEnded {
        stop_reason: String,
        completed_normally: bool,
    },
    /// The agent self-switched mode (via `CurrentModeUpdate` notification) or a
    /// `set_mode` request was confirmed. `mode_id` is the new active mode.
    ModeChanged { mode_id: String },
    /// The agent advertised or updated its available slash commands
    /// (`AvailableCommandsUpdate`). Replaces the host's cached command list.
    AvailableCommandsChanged(Vec<crate::model::SlashCommand>),
    /// The agent's execution plan changed (`SessionUpdate::Plan`). Full replacement.
    PlanChanged(Vec<crate::model::PlanEntryView>),
    /// Session metadata changed (`SessionInfoUpdate`): the title and/or the
    /// last-activity timestamp. Each field applies additively — `Unchanged`
    /// leaves the host's cached value alone (the protocol omits fields it isn't
    /// touching), so this one event covers a title-only, timestamp-only, or
    /// combined update without a bool/`Option` soup.
    SessionInfoChanged {
        title: InfoFieldChange,
        updated_at: InfoFieldChange,
    },
    /// A non-fatal advisory message (e.g. set_mode on connect was rejected
    /// by the adapter). The session remains live; the host should log this
    /// at Warning severity without changing the session status.
    Notice(String),
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
    /// Send a `session/set_mode` request to switch the agent to the named mode.
    SetMode(String),
    /// Send a `session/set_config_option` request to change a config option
    /// (model / effort / etc.) to the given value.
    SetConfigOption { config_id: String, value: String },
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

    /// A detached handle for host-side tests: its command channel has no live
    /// receiver, so `send_prompt`/`cancel`/… become no-ops. Lets a host test
    /// hold `Some(handle)` (to exercise the "a live session is present" branches)
    /// without spawning a real agent connection.
    #[doc(hidden)]
    pub fn detached_for_test() -> Self {
        Self {
            commands: futures::channel::mpsc::unbounded().0,
            permission_parks: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
        }
    }

    /// Request a mode switch via `session/set_mode`. Returns immediately; the
    /// connection task issues the request on the next idle cycle. The agent
    /// confirms by emitting a `CurrentModeUpdate` notification, which surfaces
    /// as [`AcpEvent::ModeChanged`].
    ///
    /// A send failure means the connection task has already ended; the error is
    /// dropped because the host learns of termination via the event stream.
    pub fn set_mode(&self, mode_id: String) {
        let _ = self.commands.unbounded_send(Command::SetMode(mode_id));
    }

    /// Request a config option change via `session/set_config_option`. Returns
    /// immediately; the connection task issues the request on the next idle
    /// cycle and emits [`AcpEvent::ConfigOptionsChanged`] with the agent's
    /// updated option set.
    ///
    /// A send failure means the connection task has already ended; the error is
    /// dropped because the host learns of termination via the event stream.
    pub fn set_config_option(&self, config_id: String, value: String) {
        let _ = self
            .commands
            .unbounded_send(Command::SetConfigOption { config_id, value });
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

/// Dev-build ACP wire tap. When the `DARUDA_ACP_WIRE_LOG` environment variable
/// names a file, every raw JSON-RPC line exchanged with the adapter — client →
/// adapter (`stdin`), adapter → client (`stdout`), and the adapter's own
/// `stderr` — is appended there with a millisecond timestamp and a direction
/// marker, so tool-call / subagent traffic can be inspected off-line. Identity
/// (no tap) when the variable is unset or the file can't be opened, so the
/// shipping build never touches the wire unless explicitly asked. The app sets
/// the variable in debug builds (see `bootstrap::init_observability`); it can
/// also be set by hand to capture from any build.
fn attach_wire_log(agent: AcpAgent) -> AcpAgent {
    let Some(path) = std::env::var_os("DARUDA_ACP_WIRE_LOG") else {
        return agent;
    };
    // Open once (append) and share the handle with the debug closure; a per-line
    // reopen would thrash under streaming turns.
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(f) => Arc::new(Mutex::new(f)),
        Err(_) => return agent,
    };
    agent.with_debug(move |line, direction| {
        use std::io::Write as _;
        let marker = match direction {
            LineDirection::Stdin => "-> stdin ",
            LineDirection::Stdout => "<- stdout",
            LineDirection::Stderr => "!! stderr",
        };
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        if let Ok(mut f) = file.lock() {
            let _ = writeln!(f, "{ts} {marker} {line}");
        }
    })
}

/// Open a long-lived ACP session against `command`, rooted at `cwd`.
///
/// `initial_mode` is an optional ACP mode id (e.g. `"bypassPermissions"`) to
/// apply right after `session/new` via `session/set_mode`. The mode is applied
/// only when the adapter advertises it in the session's available modes and it
/// differs from the session's current mode; if the adapter does not support
/// modes or the id is not in the advertised list, `Connected` is emitted
/// unchanged. Pass `None` to keep whatever mode the adapter defaults to.
///
/// Spawns the protocol connection as a detached smol task and returns a handle
/// plus the event receiver. The task runs until the handle is dropped (command
/// channel closes) or the connection fails; either way the event stream then
/// reaches end-of-stream. A failure to *parse* the adapter command is reported
/// synchronously as an error here, before any task is spawned.
pub fn connect_session(
    command: AdapterCommand,
    cwd: PathBuf,
    initial_mode: Option<String>,
    resume: Option<SessionId>,
) -> Result<(AcpSessionHandle, UnboundedReceiver<AcpEvent>), AcpClientError> {
    let agent = AcpAgent::from_str(&command.0)
        .map_err(|e| AcpClientError::Command(format!("{e:?}")))
        .map(attach_wire_log)?;

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
            initial_mode,
            resume,
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

/// Launch an ACP agent from an arbitrary `command` string, provisioning a
/// Node.js runtime only when the command needs one, then open a session — the
/// entry point the host uses instead of building an [`AdapterCommand`] by hand.
///
/// When `command` is an `npx` / `node` launcher (see
/// [`crate::node::command_needs_node`]), a usable Node.js is ensured: the user's
/// system Node.js when present, otherwise a managed Node.js downloaded into
/// `node_install_dir` (see [`crate::node::ensure_node`]), and the command is
/// rewritten to run on it. Any other command (a self-contained JSON stdio config
/// or a standalone binary) is launched verbatim without touching Node.js. A
/// provisioning failure is surfaced as [`AcpClientError::Runtime`], whose
/// `Display` carries a user-facing remedy. `progress` reports runtime-prep
/// milestones so the host can show a status line during the (first-run only)
/// download.
pub fn connect_agent_session(
    command: String,
    node_install_dir: PathBuf,
    cwd: PathBuf,
    initial_mode: Option<String>,
    resume: Option<SessionId>,
    progress: &mut dyn FnMut(crate::node::NodeProgress),
) -> Result<(AcpSessionHandle, UnboundedReceiver<AcpEvent>), AcpClientError> {
    let adapter = if crate::node::command_needs_node(&command) {
        crate::node::ensure_node(&node_install_dir, progress)?.wrap_command(&command)
    } else {
        AdapterCommand(command)
    };
    connect_session(adapter, cwd, initial_mode, resume)
}

/// Drive the whole connection: handshake, session creation, then the prompt /
/// cancel select loop, until the command channel closes or the protocol fails.
async fn run_connection(
    agent: AcpAgent,
    cwd: PathBuf,
    initial_mode: Option<String>,
    resume: Option<SessionId>,
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
                // Intercept `CurrentModeUpdate` so the host sees a typed
                // `ModeChanged` event rather than the raw update. All other
                // updates are forwarded as `AcpEvent::Update`.
                let event = match notification.update {
                    SessionUpdate::CurrentModeUpdate(u) => AcpEvent::ModeChanged {
                        mode_id: u.current_mode_id.to_string(),
                    },
                    SessionUpdate::AvailableCommandsUpdate(u) => {
                        AcpEvent::AvailableCommandsChanged(
                            u.available_commands
                                .iter()
                                .map(crate::model::SlashCommand::from)
                                .collect(),
                        )
                    }
                    SessionUpdate::Plan(p) => AcpEvent::PlanChanged(
                        p.entries
                            .iter()
                            .map(crate::model::PlanEntryView::from)
                            .collect(),
                    ),
                    // Title and last-activity timestamp. The adapter pushes both
                    // together at turn-end, but each field is mapped independently
                    // so a title-only or timestamp-only update (per the protocol's
                    // per-field `MaybeUndefined`) is also handled correctly.
                    SessionUpdate::SessionInfoUpdate(u) => AcpEvent::SessionInfoChanged {
                        title: u.title.into(),
                        updated_at: u.updated_at.into(),
                    },
                    // The agent pushed a config-option change it made itself
                    // (e.g. a fast-mode toggle, or effort reconciliation after a
                    // mode downgrade) — not a reply to our `set_config_option`.
                    // Carries the full option set, so reuse the same
                    // ConfigOptionsChanged full-replace the request path emits;
                    // without this the model/effort/mode chips show a stale value
                    // after any agent-driven change.
                    SessionUpdate::ConfigOptionUpdate(u) => AcpEvent::ConfigOptionsChanged(
                        config_options_from_protocol(&u.config_options),
                    ),
                    update => AcpEvent::Update(Box::new(update)),
                };
                let _ = notif_tx.unbounded_send(event);
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

            // New session, or resume an existing one via session/load. A load
            // replays the prior conversation as session/update notifications
            // (handled by the notification handler above) before the response
            // resolves, so the host rebuilds its items exactly as for a live
            // turn. Both paths yield the same (session_id, modes, config_options)
            // and share the set_mode + Connected tail below.
            let (session_id, mut modes, config_options): (
                SessionId,
                Option<ModeStateView>,
                Vec<ConfigOptionView>,
            ) = match resume {
                Some(id) => {
                    let loaded = connection
                        .send_request(LoadSessionRequest::new(id.clone(), cwd))
                        .block_task()
                        .await?;
                    (
                        id,
                        loaded.modes.as_ref().map(Into::into),
                        config_options_from_protocol(
                            loaded.config_options.as_deref().unwrap_or(&[]),
                        ),
                    )
                }
                None => {
                    let new_session = connection
                        .send_request(NewSessionRequest::new(cwd))
                        .block_task()
                        .await?;
                    (
                        new_session.session_id.clone(),
                        new_session.modes.as_ref().map(Into::into),
                        config_options_from_protocol(
                            new_session.config_options.as_deref().unwrap_or(&[]),
                        ),
                    )
                }
            };

            // Apply the configured initial mode when the adapter advertised it,
            // the requested id is in the available list, and it differs from
            // the session's current mode. Skipped silently when any condition
            // is false so a misconfigured or non-advertising adapter is
            // forward-compatible.
            //
            // A set_mode failure is NON-FATAL: the session/new already
            // succeeded and the session is usable. Leave mode_state.current
            // at the adapter's real current mode (the chip will reflect that),
            // emit a Notice so the host can log it, and continue to Connected.
            if let (Some(id), Some(ref mut mode_state)) = (initial_mode, modes.as_mut()) {
                let available = mode_state.available.iter().any(|m| m.id == id);
                if available && mode_state.current != id {
                    match connection
                        .send_request(SetSessionModeRequest::new(session_id.clone(), id.clone()))
                        .block_task()
                        .await
                    {
                        Ok(_) => {
                            mode_state.current = id;
                        }
                        Err(e) => {
                            let _ = event_tx.unbounded_send(AcpEvent::Notice(format!(
                                "set_mode({id}) on connect failed — session is active in the \
                                 adapter's default mode: {e:?}"
                            )));
                        }
                    }
                }
            }

            let _ = event_tx.unbounded_send(AcpEvent::Connected {
                session_id: session_id.to_string(),
                modes,
                config_options,
            });

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
                // A mode switch while idle: issue the request and wait for the
                // next command (the agent confirms via CurrentModeUpdate). A
                // rejected switch is non-fatal (Notice), never a session kill.
                Some(Command::SetMode(id)) => {
                    send_set_mode(connection, &session_id, id, event_tx).await;
                    continue;
                }
                // A config option change while idle: issue the request and wait
                // for the next command. The response carries the updated set; a
                // failure is non-fatal (Notice).
                Some(Command::SetConfigOption { config_id, value }) => {
                    send_set_config_option(connection, &session_id, config_id, value, event_tx)
                        .await;
                    continue;
                }
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
                Some(Command::SetMode(id)) => {
                    // Mode switch mid-turn: send the request and keep awaiting
                    // the prompt response; the confirmation arrives via
                    // CurrentModeUpdate notification. Non-fatal on rejection.
                    send_set_mode(connection, session_id, id, event_tx).await;
                }
                Some(Command::SetConfigOption { config_id, value }) => {
                    // Config change mid-turn: issue it and keep awaiting the
                    // prompt response; the updated set arrives in the response,
                    // forwarded as ConfigOptionsChanged. Non-fatal on rejection.
                    send_set_config_option(connection, session_id, config_id, value, event_tx)
                        .await;
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
        completed_normally: turn_completed_normally(&stop_reason),
        stop_reason: format!("{stop_reason:?}"),
    });
    Ok(handle_dropped)
}

/// Whether a turn's stop reason represents a normal completion rather than a
/// client-initiated cancellation. Every stop reason except `Cancelled` (a normal
/// `EndTurn`, hitting `MaxTokens` / `MaxTurnRequests`, or a `Refusal`) is a turn
/// that ran to its own conclusion.
fn turn_completed_normally(sr: &StopReason) -> bool {
    !matches!(sr, StopReason::Cancelled)
}

/// Send a `session/set_mode`, downgrading a rejected switch to a non-fatal
/// [`AcpEvent::Notice`]. A mode switch is a user-initiated optional action, so
/// an adapter that rejects it must NOT tear down the live session (unlike a
/// `session/prompt` failure, which is terminal). Infallible by design: the
/// connection keeps running whatever the adapter answers. On success the agent
/// confirms the switch via a `CurrentModeUpdate` notification
/// ([`AcpEvent::ModeChanged`]).
async fn send_set_mode(
    connection: &ConnectionTo<Agent>,
    session_id: &SessionId,
    mode_id: String,
    event_tx: &UnboundedSender<AcpEvent>,
) {
    if let Err(e) = connection
        .send_request(SetSessionModeRequest::new(
            session_id.clone(),
            mode_id.clone(),
        ))
        .block_task()
        .await
    {
        let _ = event_tx.unbounded_send(AcpEvent::Notice(format!(
            "set_mode({mode_id}) failed — the session stays in its current mode: {e:?}"
        )));
    }
}

/// Send a `session/set_config_option` and broadcast the agent's updated option
/// set as [`AcpEvent::ConfigOptionsChanged`]. The protocol returns the full
/// option list in the response, so the host replaces its cache wholesale (no
/// separate notification, unlike `set_mode` which confirms via
/// `CurrentModeUpdate`).
///
/// Infallible like [`send_set_mode`]: a rejected config change is a user-driven
/// optional action, downgraded to a non-fatal [`AcpEvent::Notice`] rather than
/// propagated as an error that would end the connection.
async fn send_set_config_option(
    connection: &ConnectionTo<Agent>,
    session_id: &SessionId,
    config_id: String,
    value: String,
    event_tx: &UnboundedSender<AcpEvent>,
) {
    match connection
        .send_request(SetSessionConfigOptionRequest::new(
            session_id.clone(),
            config_id.clone(),
            value.clone(),
        ))
        .block_task()
        .await
    {
        Ok(resp) => {
            let options = config_options_from_protocol(&resp.config_options);
            let _ = event_tx.unbounded_send(AcpEvent::ConfigOptionsChanged(options));
        }
        Err(e) => {
            let _ = event_tx.unbounded_send(AcpEvent::Notice(format!(
                "set_config_option({config_id}={value}) failed — the session keeps its \
                 current value: {e:?}"
            )));
        }
    }
}

/// Map a protocol config-option list to the view model, dropping non-select
/// kinds (`from_protocol` returns `None` for them). The single conversion site
/// shared by all three sources of a full option set: the connect-time advertise
/// (`session/new` response), the `set_config_option` reply, and the agent-pushed
/// `ConfigOptionUpdate` notification.
fn config_options_from_protocol(
    options: &[agent_client_protocol::schema::v1::SessionConfigOption],
) -> Vec<ConfigOptionView> {
    options
        .iter()
        .filter_map(ConfigOptionView::from_protocol)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::PermissionOptionId;

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
        assert_eq!(views[0].current_value, "sonnet");
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
}
