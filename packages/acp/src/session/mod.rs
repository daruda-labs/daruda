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
//! resolves it through [`AcpSessionHandle::respond_permission`]. The park runs
//! in a task spawned off the handler (`connection.spawn`), never inline in it:
//! the SDK dispatches all incoming messages on one task and holds it until the
//! handler returns, so an inline await would freeze every queued update for as
//! long as the permission prompt stays open.

mod connection;
mod notifications;
mod requests;
mod turn;

use connection::*;
use notifications::*;
use requests::*;
use turn::*;

use crate::prompt::PromptInput;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AuthCapabilities, BooleanConfigOptionCapabilities, CancelNotification, ClientCapabilities,
    ClientSessionCapabilities, InitializeRequest, LoadSessionRequest, McpServer, NewSessionRequest,
    PromptRequest, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionConfigOptionValue, SessionConfigOptionsCapabilities,
    SessionId, SessionUpdate, SetSessionConfigOptionRequest, SetSessionModeRequest, StopReason,
};
#[cfg(test)]
use agent_client_protocol::schema::v1::{ContentBlock, TextContent};
use agent_client_protocol::{Agent, Client, ConnectTo, ConnectionTo, JsonRpcNotification};
use futures::FutureExt;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::channel::oneshot;
use futures::future::Either;

use crate::connection::{AcpClientError, AdapterCommand, LaunchSpec};
use crate::failure::AcpFailure;
use crate::login_method::{LoginMethod, parse_login_methods};
use crate::mode_tracker::ModeTracker;
use crate::model::{
    ConfigOptionCategoryView, ConfigOptionKindView, ConfigOptionView, ConfigValueView,
    ModeStateView, SessionCapabilitiesView,
};
use crate::native_subagents::{NativeSubagentRouter, Routed};

/// Map of in-flight permission requests awaiting a host decision: request id →
/// the oneshot sender that unparks the connection's `on_receive_request`
/// handler once the host calls [`AcpSessionHandle::respond_permission`].
type PermissionParks = Arc<Mutex<HashMap<u64, oneshot::Sender<PermissionDecision>>>>;

/// Whether failing to restore may create a different conversation.
pub enum SessionResume {
    Fresh,
    BestEffort(SessionId),
    Required(SessionId),
}

impl From<Option<SessionId>> for SessionResume {
    fn from(id: Option<SessionId>) -> Self {
        match id {
            Some(id) => Self::BestEffort(id),
            None => Self::Fresh,
        }
    }
}

impl SessionResume {
    fn resolve(
        self,
        supports_load: bool,
    ) -> Result<(Option<SessionId>, Option<String>), agent_client_protocol::Error> {
        match self {
            Self::Required(id) if supports_load => Ok((Some(id), None)),
            Self::Required(_) => Err(agent_client_protocol::Error::new(
                -32601,
                "This agent does not support session/load; the saved conversation was not replaced.",
            )),
            Self::BestEffort(id) => Ok(resolve_resume(Some(id), supports_load)),
            Self::Fresh => Ok(resolve_resume(None, supports_load)),
        }
    }
}

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

/// A milestone reached while a connect is in flight, before the session is
/// ready for prompts. Purely a progress marker for the host's status line —
/// it carries no data of its own, unlike [`crate::node::NodeProgress`] (which
/// tracks a download's byte progress).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectPhase {
    /// `initialize` request sent, awaiting the agent's capabilities reply.
    Handshaking,
    /// `session/new` sent — creating a fresh session.
    CreatingSession,
    /// `session/load` sent — resuming a persisted session id.
    LoadingSession,
    /// `session/set_mode` sent to apply the host's requested mode, on a
    /// fresh and a resumed session alike.
    ApplyingMode,
}

/// An event emitted by the live connection for the host to consume.
#[derive(Debug)]
pub enum AcpEvent {
    /// A connect milestone was reached. Only ever emitted before the matching
    /// [`AcpEvent::Connected`] or [`AcpEvent::Error`] — the host should ignore
    /// a late/stale one that arrives after either (guard on current status).
    ConnectProgress(ConnectPhase),
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
        /// Which optional session methods the agent advertised at `initialize`
        /// (`session/load` / `list` / `resume` / `close`). The host gates the
        /// matching affordances on these flags.
        capabilities: SessionCapabilitiesView,
        /// How the agent says a user can sign in, from `initialize`. Empty when
        /// it advertises none — the host then derives the login command itself.
        ///
        /// Carried on connect rather than fetched when a login is needed: the
        /// agent only offers this at `initialize`, and by the time a failure
        /// asks for a re-login the session that could have answered is the one
        /// that just failed.
        login_methods: Vec<LoginMethod>,
        /// The program the agent called itself at `initialize`
        /// (`agent_info.name`, e.g. `@agentclientprotocol/codex-acp`). `None`
        /// for an agent that reports no identity — the protocol still has the
        /// field optional. This is what decides which ACP dialect the session
        /// speaks, so the host feeds it to
        /// [`crate::adapter::adapter_for`]: the catalog id is only daruda's
        /// label and a user may register the same program under any id.
        ///
        /// An idempotent restatement of [`AcpEvent::AgentIdentified`], which
        /// carried it early enough for a resume's replayed updates.
        program: Option<String>,
    },
    /// The agent said what program it is (`initialize`'s `agent_info.name`).
    ///
    /// Emitted as soon as `initialize` answers — **before** any `session/update`
    /// can arrive. That timing is the point: a `session/load` replays the whole
    /// prior conversation as updates before [`AcpEvent::Connected`] resolves, so
    /// a host that learned the program from `Connected` would map every restored
    /// item with the wrong strategy and the live ones with the right one, giving
    /// a single transcript two dialects. `None` for an agent that reports no
    /// identity, which leaves the host's catalog id to decide.
    AgentIdentified { program: Option<String> },
    /// The agent replaced its session config option state (the protocol carries
    /// the full option set), from either source: the reply to our
    /// `set_config_option` request, or an agent-pushed `ConfigOptionUpdate`
    /// notification (a change the agent made itself — e.g. a fast-mode toggle).
    /// Either way it is a full replacement of the host's cached options.
    ConfigOptionsChanged(Vec<ConfigOptionView>),
    /// The agent refused a `set_config_option`. Non-fatal — the session
    /// keeps the value it had — but named, so a host that *required* the
    /// change can tell a refusal apart from a confirmation that simply has
    /// not arrived. A chat pane flipping a model chip wants the old
    /// behaviour (carry on); a flow node pinned to that model has to fail,
    /// and without this it can only wait out its settings budget to learn
    /// the same thing.
    ConfigOptionRejected { config_id: String, reason: String },
    /// A `session/update` notification arrived. The host folds it into its
    /// chat model via [`crate::mapping::apply_update`].
    Update(Box<SessionUpdate>),
    /// The agent reported live token/context accounting (`UsageUpdate`): the
    /// current context-window fill and optional cumulative cost. Full
    /// replacement of the host's cached usage.
    UsageChanged(crate::model::UsageView),
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
        /// What this turn cost, when the agent reported it. `None` for an agent
        /// that omits it and for a turn torn down without a reply.
        usage: Option<crate::model::TurnUsageView>,
    },
    /// A `session/prompt` returned a JSON-RPC error (e.g. the adapter hit a
    /// usage / session limit → `-32603`). This is a TURN-level failure, not a
    /// connection failure: the error is a normal response, so the ACP session
    /// stays alive. The host surfaces the message inline and keeps the session
    /// usable, so the user can re-prompt (e.g. once the limit resets) without
    /// reconnecting — distinct from the terminal [`AcpEvent::Error`].
    ///
    /// Carries the classified failure, not a message: an expired login and an
    /// organization-blocked one both arrive here and need opposite remedies.
    TurnFailed(AcpFailure),
    /// The session's mode state changed — the agent self-switched (via a
    /// `CurrentModeUpdate` notification), a `set_mode` was confirmed, or the
    /// agent re-advertised its mode list (it rebuilds one per model).
    ///
    /// Carries the whole reconciled state, not just the new id: the protocol
    /// splits "which mode" and "which modes exist" across two channels, and
    /// [`crate::mode_tracker`] folds them so the host has a single mode mirror
    /// to assign. Emitted only when the state actually changed.
    ModeChanged { state: ModeStateView },
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
    ///
    /// The body is the adapter's own diagnostic, passed through verbatim.
    /// daruda-authored advice is its own variant so the host can translate
    /// it — see [`AcpEvent::LegacyDelegation`].
    Notice(String),
    /// The agent delegated through its legacy collaboration tools, so the
    /// subagent's own tool calls will never be sent and its work cannot
    /// appear. Advisory, like [`AcpEvent::Notice`], but carries no text: the
    /// wording is daruda's own and belongs in the host's locale files.
    LegacyDelegation,
    /// A connection or protocol failure. Terminal: the connection task is
    /// ending (or has ended) when this is emitted.
    ///
    /// Carries the classified failure so the host can offer a remedy. Hosts
    /// that synthesize this event for a locally-detected failure (no protocol
    /// error behind it) build one with [`AcpFailure::unclassified`].
    Error(AcpFailure),
}

/// A command the host enqueues for the connection task to execute. Internal:
/// the handle's public methods build these.
enum Command {
    /// Send a `session/prompt` with this user text.
    Prompt(PromptInput),
    /// Send a `session/cancel` notification for the active turn.
    Cancel,
    /// Send a `session/set_mode` request to switch the agent to the named mode.
    SetMode(String),
    /// Send a `session/set_config_option` request to change a config option
    /// (model / effort / etc.) to the given value.
    SetConfigOption {
        config_id: String,
        value: ConfigValueView,
    },
}

/// Host-side handle to a live ACP session. Cloning is intentionally not derived
/// — the host holds one handle; dropping it (and thus closing the command
/// channel) tells the connection task to shut down.
pub struct AcpSessionHandle {
    commands: UnboundedSender<Command>,
    permission_parks: PermissionParks,
}

/// Test-only view of a detached handle's command channel: what the host sent
/// through it, and whether the host still holds it.
#[cfg(any(test, feature = "test-support"))]
pub struct HandleProbe(UnboundedReceiver<Command>);

#[cfg(any(test, feature = "test-support"))]
impl HandleProbe {
    /// Count the commands sent since the last call, and report whether the
    /// handle has been dropped (the channel closed behind them).
    pub fn drain(&mut self) -> (usize, bool) {
        use futures::channel::mpsc::TryRecvError;
        let mut sent = 0;
        loop {
            match self.0.try_recv() {
                Ok(_) => sent += 1,
                Err(TryRecvError::Closed) => return (sent, true),
                Err(TryRecvError::Empty) => return (sent, false),
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl AcpSessionHandle {
    /// A handle with no connection behind it, for host tests that need a
    /// "live" session without spawning an adapter.
    pub fn detached_for_test() -> (Self, HandleProbe) {
        let (commands, command_rx) = unbounded::<Command>();
        let handle = Self {
            commands,
            permission_parks: Arc::new(Mutex::new(HashMap::new())),
        };
        (handle, HandleProbe(command_rx))
    }
}

impl AcpSessionHandle {
    /// Queue a user prompt for the next turn. Returns immediately; the turn's
    /// completion surfaces as [`AcpEvent::TurnEnded`].
    ///
    /// A send failure means the connection task has already ended; the error
    /// is dropped because the host learns of termination via the event stream
    /// (an [`AcpEvent::Error`] or end-of-stream).
    pub fn send_prompt(&self, text: String) {
        self.send_prompt_with_attachments(text, Vec::new());
    }

    /// Queue immutable attachments alongside their text for one serialized turn.
    pub fn send_prompt_with_attachments(
        &self,
        text: String,
        attachments: Vec<crate::PromptAttachment>,
    ) {
        let _ = self
            .commands
            .unbounded_send(Command::Prompt(PromptInput { text, attachments }));
    }

    /// Request cancellation of the active turn via `session/cancel`. The agent
    /// still finishes flushing updates and returns a `Cancelled` stop reason,
    /// surfaced as a normal [`AcpEvent::TurnEnded`].
    pub fn cancel(&self) {
        let _ = self.commands.unbounded_send(Command::Cancel);
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
    pub fn set_config_option(&self, config_id: String, value: ConfigValueView) {
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

/// Open a long-lived ACP session against `command`, rooted at `cwd`.
///
/// `initial_modes` is a priority-ordered list of ACP mode ids (e.g.
/// `["bypassPermissions", "auto"]`) to apply via `session/set_mode` once the
/// session exists — after `session/new` and `session/load` alike. The first mode the adapter both
/// advertises and accepts wins; a candidate that is unadvertised or whose
/// `set_mode` is rejected falls through to the next, so a preferred-but-unavailable
/// mode degrades to its fallback instead of leaving the session in an arbitrary
/// state. If none apply (empty list, no advertised candidate, or the adapter
/// doesn't support modes), `Connected` is emitted with whatever mode the
/// adapter defaults to.
///
/// Spawns the protocol connection as a detached smol task and returns a handle
/// plus the event receiver. The task runs until the handle is dropped (command
/// channel closes) or the connection fails; either way the event stream then
/// reaches end-of-stream. A failure to *parse* the adapter command is reported
/// synchronously as an error here, before any task is spawned.
///
/// `agent_id` is the catalog id (e.g. `"claude"` / `"codex"`) used only to key
/// the dev-build wire-tap file — see [`crate::wire_log`]. Pass `""` when the
/// caller has no such identity (the crate's own examples).
pub fn connect_session(
    command: AdapterCommand,
    cwd: PathBuf,
    initial_modes: Vec<String>,
    resume: Option<SessionId>,
    agent_id: &str,
) -> Result<(AcpSessionHandle, UnboundedReceiver<AcpEvent>), AcpClientError> {
    connect_prepared_session(
        command.into(),
        cwd,
        None,
        initial_modes,
        resume,
        agent_id,
        Vec::new(),
    )
}

/// `mcp_servers` is what the new session may reach.
///
/// Passed at `session/new` rather than written into the agent's own config
/// files, which keeps it session-scoped: nothing to clean up if the app dies,
/// and a second session cannot inherit it.
#[allow(clippy::too_many_arguments)]
pub fn connect_prepared_session(
    prepared: crate::PreparedAdapter,
    cwd: PathBuf,
    initial_model: Option<String>,
    initial_modes: Vec<String>,
    resume: impl Into<SessionResume>,
    agent_id: &str,
    mcp_servers: Vec<McpServer>,
) -> Result<(AcpSessionHandle, UnboundedReceiver<AcpEvent>), AcpClientError> {
    let resume = resume.into();
    let agent = prepared
        .agent()
        .map(|agent| crate::wire_log::attach(agent, agent_id))?;

    let (command_tx, command_rx) = unbounded::<Command>();
    let (event_tx, event_rx) = unbounded::<AcpEvent>();
    let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));

    let handle = AcpSessionHandle {
        commands: command_tx,
        permission_parks: permission_parks.clone(),
    };

    let task_event_tx = event_tx.clone();
    smol::spawn(async move {
        // Protect files even if the host drops its handle before ACP shuts down.
        let _installation = prepared;
        if let Err(err) = run_connection(
            agent,
            cwd,
            initial_model,
            initial_modes,
            resume,
            mcp_servers,
            command_rx,
            task_event_tx.clone(),
            permission_parks,
        )
        .await
        {
            // Terminal failure: surface it, then let the channel drop close the
            // stream. `unbounded_send` only fails if the host stopped reading.
            let _ = task_event_tx.unbounded_send(AcpEvent::Error(err.into_failure()));
        }
    })
    .detach();

    Ok((handle, event_rx))
}

/// Launch an ACP agent from a [`LaunchSpec`], provisioning a Node.js runtime
/// only when its command needs one, then open a session — the entry point the
/// host uses instead of building an [`AdapterCommand`] by hand.
///
/// When the command is an `npx` / `node` launcher (see
/// [`crate::node::command_needs_node`]), a usable Node.js is ensured: the user's
/// system Node.js when present, otherwise a managed Node.js downloaded into
/// `node_install_dir` (see [`crate::node::ensure_node`]), and the command is
/// rewritten to run on it. Any other command (a self-contained JSON stdio config
/// or a standalone binary) is launched verbatim without touching Node.js. A
/// provisioning failure is surfaced as [`AcpClientError::Runtime`], whose
/// `Display` carries a user-facing remedy. `progress` reports runtime-prep
/// milestones so the host can show a status line during the (first-run only)
/// download. `agent_id` is the catalog id (e.g. `"claude"` / `"codex"`) —
/// see [`connect_session`]'s doc comment for what it's used for.
///
/// Runtime selection and [`LaunchSpec::strip_env`] both live in
/// [`crate::launch_env::prepare_adapter_command`], which applies the strip
/// once to whichever runtime shape it selected — node detection has to read the
/// *unstripped* command, or the `/usr/bin/env` form would mask the launcher
/// token and skip provisioning.
#[allow(clippy::too_many_arguments)] // Thin pass-through to `connect_session` — bundling wraps callers more than it saves.
pub fn connect_agent_session(
    launch: LaunchSpec,
    node_install_dir: PathBuf,
    cwd: PathBuf,
    initial_modes: Vec<String>,
    resume: Option<SessionId>,
    agent_id: &str,
    progress: &mut dyn FnMut(crate::node::NodeProgress),
) -> Result<(AcpSessionHandle, UnboundedReceiver<AcpEvent>), AcpClientError> {
    let adapter = crate::launch_env::prepare_adapter(
        &launch,
        &node_install_dir,
        progress,
        &crate::preparation::PreparationContext::default(),
    )?;
    connect_prepared_session(
        adapter,
        cwd,
        None,
        initial_modes,
        resume,
        agent_id,
        Vec::new(),
    )
}

/// [`connect_agent_session`] with one model to negotiate before the mode and
/// before [`AcpEvent::Connected`]. The model is applied only when the agent
/// advertises it; an unavailable or rejected value leaves the adapter's own
/// selection standing and does not fail the otherwise-usable session.
#[allow(clippy::too_many_arguments)] // Additive host-specific entry point; keeps the established API unchanged.
pub fn connect_agent_session_with_model(
    launch: LaunchSpec,
    node_install_dir: PathBuf,
    cwd: PathBuf,
    initial_model: Option<String>,
    initial_modes: Vec<String>,
    resume: Option<SessionId>,
    agent_id: &str,
    mcp_servers: Vec<McpServer>,
    progress: &mut dyn FnMut(crate::node::NodeProgress),
) -> Result<(AcpSessionHandle, UnboundedReceiver<AcpEvent>), AcpClientError> {
    let adapter = crate::launch_env::prepare_adapter(
        &launch,
        &node_install_dir,
        progress,
        &crate::preparation::PreparationContext::default(),
    )?;
    connect_prepared_session(
        adapter,
        cwd,
        initial_model,
        initial_modes,
        resume,
        agent_id,
        mcp_servers,
    )
}

/// Build the stdio MCP server entry a session should be handed.
///
/// Here rather than at the call site so the app never names
/// `agent_client_protocol` — the crate boundary CLAUDE.md draws is that ACP
/// types live behind `daruda_acp`.
pub fn stdio_mcp_server(
    name: String,
    command: PathBuf,
    args: Vec<String>,
    env: Vec<(String, String)>,
) -> McpServer {
    use agent_client_protocol::schema::v1::{EnvVariable, McpServerStdio};

    McpServer::Stdio(
        McpServerStdio::new(name, command).args(args).env(
            env.into_iter()
                .map(|(name, value)| EnvVariable::new(name, value))
                .collect(),
        ),
    )
}

/// Vendor-private `_meta` flag that makes the agent attach an executable
/// `command` + `args` pair to each advertised login method. Not in the ACP
/// spec; read from adapter source and confirmed against a live capture.
pub const TERMINAL_AUTH_META_KEY: &str = "terminal-auth";

#[cfg(test)]
mod tests;
