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

use std::collections::HashMap;
use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AuthCapabilities, BooleanConfigOptionCapabilities, CancelNotification, ClientCapabilities,
    ClientSessionCapabilities, ContentBlock, InitializeRequest, LoadSessionRequest,
    NewSessionRequest, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionConfigOptionValue,
    SessionConfigOptionsCapabilities, SessionId, SessionUpdate, SetSessionConfigOptionRequest,
    SetSessionModeRequest, StopReason, TextContent,
};
use agent_client_protocol::{
    AcpAgent, Agent, Client, ConnectTo, ConnectionTo, JsonRpcNotification,
};
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
    /// `session/set_mode` sent to apply the configured initial mode on a
    /// freshly created session, or the persisted `restore_mode` on a resumed
    /// one.
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
    Notice(String),
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
    Prompt(String),
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
/// `["bypassPermissions", "auto"]`) to apply right after a *fresh*
/// `session/new` via `session/set_mode`. The first mode the adapter both
/// advertises and accepts wins; a candidate that is unadvertised or whose
/// `set_mode` is rejected falls through to the next, so a preferred-but-unavailable
/// mode degrades to its fallback instead of leaving the session in an arbitrary
/// state. If none apply (empty list, no advertised candidate, or the adapter
/// doesn't support modes), `Connected` is emitted with whatever mode the
/// adapter defaults to.
///
/// `restore_mode` is the single mode id the host last saw this *resumed*
/// session in (persisted across restarts), applied the same way on a real
/// `session/load` instead of `initial_modes`. Some adapters recompute their
/// advertised mode from static config on every process launch rather than the
/// resumed session's actual last mode (see the `restore_mode` application
/// site in [`run_connection`] for the specifics); this is the host's
/// workaround for that.
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
    restore_mode: Option<String>,
    resume: Option<SessionId>,
    agent_id: &str,
) -> Result<(AcpSessionHandle, UnboundedReceiver<AcpEvent>), AcpClientError> {
    connect_session_inner(
        command,
        cwd,
        None,
        initial_modes,
        restore_mode,
        resume,
        agent_id,
    )
}

fn connect_session_inner(
    command: AdapterCommand,
    cwd: PathBuf,
    initial_model: Option<String>,
    initial_modes: Vec<String>,
    restore_mode: Option<String>,
    resume: Option<SessionId>,
    agent_id: &str,
) -> Result<(AcpSessionHandle, UnboundedReceiver<AcpEvent>), AcpClientError> {
    let agent = AcpAgent::from_str(&command.0)
        .map_err(|e| AcpClientError::Command(format!("{e:?}")))
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
        if let Err(err) = run_connection(
            agent,
            cwd,
            initial_model,
            initial_modes,
            restore_mode,
            resume,
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
    restore_mode: Option<String>,
    resume: Option<SessionId>,
    agent_id: &str,
    progress: &mut dyn FnMut(crate::node::NodeProgress),
) -> Result<(AcpSessionHandle, UnboundedReceiver<AcpEvent>), AcpClientError> {
    let adapter = crate::launch_env::prepare_adapter_command(&launch, &node_install_dir, progress)?;
    connect_session(adapter, cwd, initial_modes, restore_mode, resume, agent_id)
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
    restore_mode: Option<String>,
    resume: Option<SessionId>,
    agent_id: &str,
    progress: &mut dyn FnMut(crate::node::NodeProgress),
) -> Result<(AcpSessionHandle, UnboundedReceiver<AcpEvent>), AcpClientError> {
    let adapter = crate::launch_env::prepare_adapter_command(&launch, &node_install_dir, progress)?;
    connect_session_inner(
        adapter,
        cwd,
        initial_model,
        initial_modes,
        restore_mode,
        resume,
        agent_id,
    )
}

/// What this client advertises at `initialize`.
///
/// Capabilities here are strictly opt-in: an agent may only use a feature the
/// client claims. `session.configOptions.boolean` is what lets an agent send a
/// native boolean toggle (e.g. Claude's "Fast mode") instead of degrading it to
/// a two-value select — see [`ConfigOptionKindView::Boolean`]. Advertise a
/// capability only once the host actually renders it.
///
/// Deliberately **not** advertised: `_meta.terminal_output`, claude-agent-acp's
/// own non-standard switch. Setting it makes a shell tool's result *content-less*
/// (`content: [{type:"terminal"}]`) and moves the bytes to
/// `_meta.terminal_output.data`; unset, the adapter returns a fenced
/// ```` ```console ```` text block. The same one flag also gates
/// `_meta.terminal_exit` — the exit badge's only source — so both stand or fall
/// together (see [`crate::adapter::TERMINAL_OUTPUT_META_KEY`]).
///
/// The parsing side is implemented and unit-tested against the three-notification
/// sequence, but the shape is read from adapter *source*
/// (`dist/acp-agent.js` + `dist/tools.js`, `@agentclientprotocol/claude-agent-acp`
/// 0.62.0), not from a wire capture. Turn it on only after a live capture
/// confirms that sequence — flipping it blind would degrade Bash rendering.
///
/// The standard `.terminal(true)` capability is likewise NOT claimed: it promises
/// the whole `terminal/*` method family, which this client does not implement.
///
/// `auth.terminal` IS claimed, and is a different promise: it only tells the
/// agent it may *list* terminal-type login methods at `initialize`. There is no
/// RPC behind it — the entries are a recipe the host runs in a terminal it
/// already owns. Without it `authMethods` comes back empty and the host is left
/// deriving the login command itself.
fn client_capabilities() -> ClientCapabilities {
    // Vendor-private companion to `auth.terminal`: with it the agent attaches
    // `_meta["terminal-auth"]` to each login method, carrying the resolved
    // interpreter path and full argv. Without it only `args` arrives and the
    // host has to re-derive which binary to run them on — the exact derivation
    // (system vs managed Node.js) this crate already does once at connect and
    // would otherwise have to repeat.
    // Top-level `_meta`, NOT `auth._meta`: the agent reads
    // `clientCapabilities._meta["terminal-auth"]`. Nesting it under `auth`
    // alongside the sibling flag looks right and is silently ignored.
    let mut meta = agent_client_protocol::schema::v1::Meta::new();
    meta.insert(
        TERMINAL_AUTH_META_KEY.to_owned(),
        serde_json::Value::Bool(true),
    );
    // Switches a supporting adapter out of flattening a spawned subagent's work
    // into this session and into announcing it as a child session instead. Safe
    // to claim only because `crate::native_subagents` normalizes that traffic
    // back into the flat tool hierarchy — without the router the child's whole
    // run would arrive as updates this schema version cannot parse, which the
    // SDK logs and drops.
    meta.insert(
        crate::native_subagents::JETBRAINS_META_KEY.to_owned(),
        crate::native_subagents::air_capabilities_meta(),
    );
    ClientCapabilities::new()
        .meta(meta)
        .auth(AuthCapabilities::new().terminal(true))
        .session(ClientSessionCapabilities::new().config_options(
            SessionConfigOptionsCapabilities::new().boolean(BooleanConfigOptionCapabilities::new()),
        ))
}

/// Vendor-private `_meta` flag that makes the agent attach an executable
/// `command` + `args` pair to each advertised login method. Not in the ACP
/// spec; read from adapter source and confirmed against a live capture.
pub const TERMINAL_AUTH_META_KEY: &str = "terminal-auth";

/// Wall-clock budget for `initialize` and — on a *fresh* session —
/// `session/new` and the optional `set_mode`. Without this, a hung adapter
/// (e.g. an SSH-wrapped remote command stuck on a silent auth prompt, or a
/// dead network) parks `block_task().await` forever: no event ever reaches
/// the host, so the connecting status never resolves. `prompt_loop` is
/// deliberately NOT covered — a live session with no traffic is normal, not a
/// hang. `session/load` (a resume) is NOT covered by this budget — see
/// [`CONNECT_RESUME_LOAD_TIMEOUT`]: these three requests carry no bulk
/// payload and a healthy adapter answers in milliseconds, so a generous but
/// still-bounded 60s is appropriate.
const CONNECT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

/// Wall-clock budget for `session/load` alone. Separate from
/// [`CONNECT_HANDSHAKE_TIMEOUT`] because a resume's response only arrives
/// after the adapter has replayed the *entire* prior conversation as
/// `session/update` notifications first — a large history, or a slow link
/// (e.g. the SSH-wrapped remote agent), can legitimately take much longer
/// than a fresh `session/new`'s near-instant reply without being hung. Still
/// bounded, so a genuinely stuck load (the same class of bug this whole
/// timeout mechanism exists for) doesn't strand the pane forever.
const CONNECT_RESUME_LOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Race `fut` against a `timeout` timer; on timeout, returns a synthetic
/// protocol error naming `what` so the host's error banner names the stuck
/// step rather than a generic failure. `fut` must resolve on its own within
/// the timeout — cancellation here only stops *waiting* for it (the future is
/// dropped), it does not abort a stuck subprocess.
async fn with_connect_timeout<T>(
    what: &str,
    timeout: Duration,
    fut: impl Future<Output = Result<T, agent_client_protocol::Error>>,
) -> Result<T, agent_client_protocol::Error> {
    // ALLOW: this crate is GPUI-free (see crates/daruda_acp/CLAUDE.md) and has
    // no BackgroundExecutor to time on; smol::Timer is the only timer source
    // available here. Tests pass a short explicit `timeout`, so this stays
    // deterministic.
    #[allow(clippy::disallowed_methods)]
    let timer = smol::Timer::after(timeout);
    match futures::future::select(Box::pin(fut), Box::pin(timer)).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => Err(agent_client_protocol::Error::new(
            -32603,
            format!(
                "{what} timed out after {}s — check the agent command and network reachability \
                 (e.g. SSH host connectivity) and retry",
                timeout.as_secs()
            ),
        )),
    }
}

/// `session/update`, received as a raw payload instead of a typed one.
///
/// The typed `SessionNotification` rejects any `sessionUpdate` this schema
/// version has no variant for — and the SDK logs a rejected notification and
/// moves on (`jsonrpc/incoming_actor.rs`: "Notification errors are logged
/// without replying"), so the update is not an error the host ever sees, it
/// simply never happened. A draft-protocol update — a native subagent session's
/// traffic — would therefore vanish in silence. Taking `update` as JSON lets
/// [`NativeSubagentRouter`] decide what it is; every standard update still ends
/// up in the same typed [`SessionUpdate`] it always did.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, JsonRpcNotification)]
#[notification(method = "session/update")]
#[serde(rename_all = "camelCase")]
struct CompatSessionNotification {
    /// Which session this update belongs to. The typed handler discarded this;
    /// with native subagents it is the only thing distinguishing a child's tool
    /// call from the main agent's.
    session_id: SessionId,
    update: serde_json::Value,
    #[serde(default, rename = "_meta")]
    #[allow(dead_code, reason = "Part of the wire shape; nothing reads it yet.")]
    meta: Option<agent_client_protocol::schema::v1::Meta>,
}

/// Fold one standard `session/update` into the host's event stream.
///
/// The two mode-bearing updates go through the tracker (0..2 events out); the
/// rest map 1:1. Shared by root traffic and by the native subagent router's
/// normalized output, so a subagent's tool calls take exactly the path the main
/// agent's take.
fn forward_session_update(
    update: SessionUpdate,
    mode_tracker: &ModeTracker,
    tx: &UnboundedSender<AcpEvent>,
) {
    let update = match update {
        SessionUpdate::CurrentModeUpdate(u) => {
            if let Some(state) = mode_tracker.apply_current_mode(u.current_mode_id.to_string()) {
                let _ = tx.unbounded_send(AcpEvent::ModeChanged { state });
            }
            return;
        }
        // The agent pushed a config-option change it made itself (e.g. a
        // fast-mode toggle, or effort reconciliation after a mode downgrade) —
        // not a reply to our `set_config_option`. Carries the full option set,
        // so reuse the same ConfigOptionsChanged full-replace the request path
        // emits; without this the model/effort chips show a stale value after
        // any agent-driven change.
        SessionUpdate::ConfigOptionUpdate(u) => {
            send_config_options_fold(
                mode_tracker,
                config_options_from_protocol(&u.config_options),
                tx,
            );
            return;
        }
        other => other,
    };
    let event = match update {
        SessionUpdate::AvailableCommandsUpdate(u) => AcpEvent::AvailableCommandsChanged(
            u.available_commands
                .iter()
                .map(crate::model::SlashCommand::from)
                .collect(),
        ),
        SessionUpdate::Plan(p) => AcpEvent::PlanChanged(
            p.entries
                .iter()
                .map(crate::model::PlanEntryView::from)
                .collect(),
        ),
        // Title and last-activity timestamp. The adapter pushes both together at
        // turn-end, but each field is mapped independently so a title-only or
        // timestamp-only update (per the protocol's per-field `MaybeUndefined`)
        // is also handled correctly.
        SessionUpdate::SessionInfoUpdate(u) => AcpEvent::SessionInfoChanged {
            title: u.title.into(),
            updated_at: u.updated_at.into(),
        },
        // Live context-window / cost accounting. Surfaced as a typed event (like
        // mode / plan / config) rather than raw `Update` so the host renders a
        // context meter without parsing protocol types. Distinct from the CLI's
        // cumulative Usage tab: this is the current context fill.
        SessionUpdate::UsageUpdate(u) => AcpEvent::UsageChanged(crate::model::UsageView::from(&u)),
        update => AcpEvent::Update(Box::new(update)),
    };
    let _ = tx.unbounded_send(event);
}

/// Drive the whole connection: handshake, session creation, then the prompt /
/// cancel select loop, until the command channel closes or the protocol fails.
///
/// Generic over the transport (production passes the subprocess-spawning
/// [`AcpAgent`]) so tests can wire an in-process fake agent — an SDK
/// `Agent.builder()` implements `ConnectTo<Client>` too — and drive this exact
/// code path deterministically, dispatch semantics included.
#[allow(clippy::too_many_arguments)] // Internal connection state threaded through one call — bundling wraps callers more than it saves.
async fn run_connection(
    agent: impl ConnectTo<Client> + 'static,
    cwd: PathBuf,
    initial_model: Option<String>,
    initial_modes: Vec<String>,
    restore_mode: Option<String>,
    resume: Option<SessionId>,
    command_rx: UnboundedReceiver<Command>,
    event_tx: UnboundedSender<AcpEvent>,
    permission_parks: PermissionParks,
) -> Result<(), AcpClientError> {
    let notif_tx = event_tx.clone();
    let perm_event_tx = event_tx.clone();
    let next_permission_id = Arc::new(AtomicU64::new(0));
    // Owns the session's mode state for the whole connection; see
    // `crate::mode_tracker` for why mode can't be forwarded as it arrives.
    let mode_tracker = ModeTracker::default();
    let notif_mode_tracker = mode_tracker.clone();
    // Holds this connection's subagent session graph. Shared with the
    // notification handler, which is registered before `initialize` answers —
    // hence the handle rather than a value.
    let router = Arc::new(Mutex::new(NativeSubagentRouter::default()));
    let notif_router = router.clone();

    agent_client_protocol::Client
        .builder()
        .on_receive_notification(
            async move |notification: CompatSessionNotification, _cx| {
                // Routing happens before anything is typed: a native subagent's
                // update has no variant in this schema version, and the SDK
                // drops what it cannot parse (logged, never surfaced), so the
                // child's whole run would vanish without this hop.
                let routed = notif_router
                    .lock()
                    .expect("native subagent router mutex poisoned")
                    .route(&notification.session_id.0, &notification.update);
                match routed {
                    Routed::Standard(update) => {
                        forward_session_update(*update, &notif_mode_tracker, &notif_tx);
                    }
                    Routed::Normalized(updates) => {
                        for update in updates {
                            forward_session_update(update, &notif_mode_tracker, &notif_tx);
                        }
                    }
                    // An update kind this build does not know is reported and
                    // skipped: an agent that adds one must not be able to break
                    // a session that otherwise works.
                    Routed::Unknown { kind } => {
                        let _ = notif_tx.unbounded_send(AcpEvent::Notice(format!(
                            "Ignoring an unrecognized session update from the agent ({kind})."
                        )));
                    }
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            {
                let permission_parks = permission_parks.clone();
                let next_permission_id = next_permission_id.clone();
                let perm_event_tx = perm_event_tx.clone();
                async move |request: RequestPermissionRequest, responder, connection| {
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

                    // Park in a spawned task, NOT in this handler: the SDK runs
                    // handlers inline on the connection's single dispatch task
                    // ("the server will not process new messages until this
                    // handler returns"), so awaiting the host's decision here
                    // would freeze every update queued behind the request —
                    // streaming, tool progress, a second permission request —
                    // until the user answers. Measured by
                    // `a_parked_permission_request_does_not_stall_update_dispatch`.
                    // If the sender is dropped (host went away / id reaped on
                    // shutdown), default to Cancelled — deny by absence.
                    connection.spawn(async move {
                        let decision = decision_rx.await.unwrap_or(PermissionDecision::Cancelled);
                        responder.respond(decision.into_response())
                    })
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, move |connection: ConnectionTo<Agent>| async move {
            // Each handshake step gets its own deadline rather than one budget
            // for the whole sequence, because `session/load` is structurally
            // different: its response only arrives after the adapter replays
            // the *entire* prior conversation, which can legitimately take far
            // longer than `initialize`/`session/new`/`set_mode`'s near-instant
            // replies without being hung — see `CONNECT_RESUME_LOAD_TIMEOUT`.
            // `prompt_loop` below is deliberately outside every timeout here —
            // a live, quiet session is normal.
            let _ = event_tx.unbounded_send(AcpEvent::ConnectProgress(ConnectPhase::Handshaking));
            let (capabilities, login_methods, program, resume, fresh) =
                with_connect_timeout("initialize", CONNECT_HANDSHAKE_TIMEOUT, async {
                    let init = connection
                        .send_request(
                            InitializeRequest::new(ProtocolVersion::V1)
                                .client_capabilities(client_capabilities()),
                        )
                        .block_task()
                        .await?;
                    let capabilities = session_capabilities_from_protocol(&init.agent_capabilities);
                    let login_methods = parse_login_methods(&init.auth_methods);
                    let program = init.agent_info.map(|info| info.name);

                    // Gate the requested resume on advertised `session/load` support:
                    // downgrade to a fresh session (with a Notice) when the agent can't
                    // replay history, so a resume against a non-load agent no longer
                    // fails the whole connect.
                    let (resume, resume_notice) = resolve_resume(resume, capabilities.load);
                    if let Some(notice) = resume_notice {
                        let _ = event_tx.unbounded_send(AcpEvent::Notice(notice));
                    }
                    // Whether this connect ends up creating a *fresh* session (either no
                    // resume was requested, or a requested resume was downgraded because
                    // the agent doesn't advertise `session/load`). The configured initial
                    // mode is applied only on a fresh session; a real load preserves the
                    // resumed session's own mode.
                    let fresh = resume.is_none();
                    Ok((capabilities, login_methods, program, resume, fresh))
                })
                .await?;
            // Before the branch below: a `session/load` inside it replays the
            // conversation as updates, and those must map under this program.
            let _ = event_tx.unbounded_send(AcpEvent::AgentIdentified {
                program: program.clone(),
            });
            // Same ordering requirement as the event above, for the same
            // reason: a `session/load` below replays under this program, and
            // the router reads that dialect's `_meta` to tell a subagent's
            // answer from its preamble.
            router
                .lock()
                .expect("native subagent router mutex poisoned")
                .set_adapter(crate::adapter::adapter_for(program.as_deref(), ""));

            // New session, or resume an existing one via session/load. A load
            // replays the prior conversation as session/update notifications
            // (handled by the notification handler above) before the response
            // resolves, so the host rebuilds its items exactly as for a live
            // turn. Both paths yield the same (session_id, modes, config_options)
            // and share the set_mode + Connected tail below.
            let (session_id, mut modes, mut config_options): (
                SessionId,
                Option<ModeStateView>,
                Vec<ConfigOptionView>,
            ) = match resume {
                Some(id) => {
                    let _ = event_tx
                        .unbounded_send(AcpEvent::ConnectProgress(ConnectPhase::LoadingSession));
                    with_connect_timeout("session/load", CONNECT_RESUME_LOAD_TIMEOUT, async {
                        let loaded = connection
                            .send_request(LoadSessionRequest::new(id.clone(), cwd))
                            .block_task()
                            .await?;
                        Ok((
                            id,
                            loaded.modes.as_ref().map(Into::into),
                            config_options_from_protocol(
                                loaded.config_options.as_deref().unwrap_or(&[]),
                            ),
                        ))
                    })
                    .await?
                }
                None => {
                    let _ = event_tx
                        .unbounded_send(AcpEvent::ConnectProgress(ConnectPhase::CreatingSession));
                    with_connect_timeout("session/new", CONNECT_HANDSHAKE_TIMEOUT, async {
                        let new_session = connection
                            .send_request(NewSessionRequest::new(cwd))
                            .block_task()
                            .await?;
                        Ok((
                            new_session.session_id.clone(),
                            new_session.modes.as_ref().map(Into::into),
                            config_options_from_protocol(
                                new_session.config_options.as_deref().unwrap_or(&[]),
                            ),
                        ))
                    })
                    .await?
                }
            };

            // A model can rebuild the mode list, so settle it first. This also
            // happens before `Connected`, which is the host's gate for draining
            // prompts queued during the handshake.
            apply_initial_model(
                &connection,
                &session_id,
                initial_model.as_deref(),
                &mut modes,
                &mut config_options,
                &event_tx,
            )
            .await;

            // Apply the configured initial mode on a *fresh* session (including a
            // resume downgraded to session/new): `initial_modes`, a
            // priority-ordered candidate list. On a *real* `session/load`, try
            // `restore_mode` instead — the single mode id the host last saw this
            // session in.
            //
            // WORKAROUND: the protocol lets `session/load`'s response carry the
            // resumed session's real mode, so in principle this candidate loop
            // should be unnecessary on a resume. But `claude-agent-acp` never
            // persists mode per session — it recomputes `permissionMode` from
            // `settings.json` on every process launch — so the value the load
            // response reports is the settings default, not the session's actual
            // last mode. Root cause is upstream (`claude-agent-acp`'s
            // `createSession`), out of scope here; `restore_mode` (persisted by
            // the host itself) papers over it until that adapter fixes it.
            //
            // Either way: try each candidate in turn and stop at the first the
            // adapter both advertises and accepts. A candidate that is not
            // advertised is skipped without a request; one whose set_mode is
            // rejected falls through to the next, so a preferred-but-unavailable
            // mode (e.g. `bypassPermissions`) degrades to its fallback (`auto`)
            // rather than leaving the session in an arbitrary state.
            //
            // Every candidate failing is NON-FATAL: the session already
            // succeeded and is usable. Leave mode_state.current at the adapter's
            // real current mode (the chip reflects that), emit a Notice so the
            // host can log it, and continue to Connected.
            let mode_candidates: Vec<String> = if fresh {
                initial_modes
            } else {
                restore_mode.into_iter().collect()
            };
            if let Some(mode_state) = modes.as_mut() {
                let mut applied = false;
                let mut last_reject: Option<(String, String)> = None;
                for id in &mode_candidates {
                    // Not advertised — this candidate can't apply; try the next.
                    if !mode_state.available.iter().any(|m| &m.id == id) {
                        continue;
                    }
                    // Already in this mode — nothing to send, we're done.
                    if &mode_state.current == id {
                        applied = true;
                        break;
                    }
                    let _ = event_tx
                        .unbounded_send(AcpEvent::ConnectProgress(ConnectPhase::ApplyingMode));
                    let set_mode_result = with_connect_timeout(
                        "session/set_mode",
                        CONNECT_HANDSHAKE_TIMEOUT,
                        connection
                            .send_request(SetSessionModeRequest::new(
                                session_id.clone(),
                                id.clone(),
                            ))
                            .block_task(),
                    )
                    .await;
                    match set_mode_result {
                        Ok(_) => {
                            mode_state.current = id.clone();
                            applied = true;
                            break;
                        }
                        // Rejected — remember it and fall through to the fallback.
                        Err(e) => last_reject = Some((id.clone(), format!("{e:?}"))),
                    }
                }
                // Only notice when a candidate was actually attempted and rejected
                // and no later candidate applied — a purely non-advertised list is
                // forward-compatible silence (matches a non-modes adapter).
                if !applied && let Some((id, err)) = last_reject {
                    let _ = event_tx.unbounded_send(AcpEvent::Notice(format!(
                        "set_mode({id}) on connect failed — session is active in the \
                         adapter's default mode: {err}"
                    )));
                }
            }

            // Hand the (post-`set_mode`) state to the tracker before any
            // mode-bearing traffic can reach the host: from here on it is the
            // one owner, and `Connected` is the last place mode arrives by any
            // other route. Seeding `None` marks a modeless agent permanently
            // inert, so mode affordances stay a connect-time decision.
            mode_tracker.seed(modes.clone());

            let _ = event_tx.unbounded_send(AcpEvent::Connected {
                session_id: session_id.to_string(),
                modes,
                // Mode is carried by `modes` above; strip the duplicate so the
                // host has exactly one representation of it (mirrors what
                // `send_config_options_fold` does for every later set).
                config_options: crate::mode_tracker::strip_mode_options(config_options),
                capabilities,
                login_methods,
                program,
            });

            prompt_loop(
                &connection,
                session_id,
                command_rx,
                &event_tx,
                &mode_tracker,
            )
            .await?;
            Ok(())
        })
        .await
        .map_err(|e| AcpClientError::Protocol(AcpFailure::classify(&e)))?;

    Ok(())
}

/// Apply a host-requested model during the handshake. Optional by design: a
/// missing choice or a refusal leaves the adapter's current model standing,
/// matching the AgentChat default semantics while still guaranteeing that any
/// accepted switch completes before the first prompt can be sent.
async fn apply_initial_model(
    connection: &ConnectionTo<Agent>,
    session_id: &SessionId,
    wanted: Option<&str>,
    modes: &mut Option<ModeStateView>,
    config_options: &mut Vec<ConfigOptionView>,
    event_tx: &UnboundedSender<AcpEvent>,
) {
    let Some(wanted) = wanted.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let Some(option) = config_options
        .iter()
        .find(|option| option.category == ConfigOptionCategoryView::Model)
    else {
        return;
    };
    let ConfigOptionKindView::Select {
        current_value,
        options,
    } = &option.kind
    else {
        return;
    };
    if current_value == wanted || !options.iter().any(|choice| choice.value == wanted) {
        return;
    }
    let config_id = option.id.clone();
    let request = SetSessionConfigOptionRequest::new(
        session_id.clone(),
        config_id.clone(),
        SessionConfigOptionValue::value_id(wanted.to_string()),
    );
    match with_connect_timeout(
        "session/set_config_option(model)",
        CONNECT_HANDSHAKE_TIMEOUT,
        connection.send_request(request).block_task(),
    )
    .await
    {
        Ok(response) => {
            let updated = config_options_from_protocol(&response.config_options);
            if let Some(updated_modes) = ModeStateView::from_config_options(&updated) {
                *modes = Some(updated_modes);
            }
            *config_options = updated;
        }
        Err(error) => {
            let _ = event_tx.unbounded_send(AcpEvent::Notice(format!(
                "set_config_option({config_id}={wanted}) on connect failed — the session uses \
                 the adapter's current model: {error:?}"
            )));
        }
    }
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
    mode_tracker: &ModeTracker,
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
                    send_set_config_option(
                        connection,
                        &session_id,
                        config_id,
                        value,
                        event_tx,
                        mode_tracker,
                    )
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
            mode_tracker,
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
    mode_tracker: &ModeTracker,
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
            resp = response => match resp {
                Ok(r) => break r.stop_reason,
                Err(e) => {
                    // A `session/prompt` that returns a JSON-RPC error (e.g. the
                    // adapter hit a usage / session limit → `-32603`) is a
                    // TURN-level failure, NOT a connection failure: the error is
                    // a normal response, so the stdio connection stays alive.
                    // Surface it as `TurnFailed` and let `prompt_loop` continue,
                    // so the session stays usable and the user can re-prompt
                    // (e.g. once the limit resets) without reconnecting. This is
                    // the one prompt-error path that must NOT propagate `?` and
                    // tear the whole session down.
                    //
                    // A genuine connection death also surfaces here as an `Err`
                    // (the library resolves the pending request with an internal
                    // error when the transport closes); the connection's
                    // background task then ends and `run_connection` returns,
                    // emitting a terminal `Error` on top of this. Re-prompting a
                    // dead connection just yields another immediate `TurnFailed`
                    // — never a hang, since the request errors at once.
                    let _ = event_tx.unbounded_send(AcpEvent::TurnFailed(AcpFailure::classify(&e)));
                    return Ok(handle_dropped);
                }
            },
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
                    send_set_config_option(
                        connection,
                        session_id,
                        config_id,
                        value,
                        event_tx,
                        mode_tracker,
                    )
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
    value: ConfigValueView,
    event_tx: &UnboundedSender<AcpEvent>,
    mode_tracker: &ModeTracker,
) {
    // The wire form is kind-specific: a select's value id serializes as a bare
    // string (`ValueId` is the untagged variant), a boolean as a tagged
    // `{"type":"boolean","value":…}`. Sending the wrong one is rejected.
    let protocol_value = match &value {
        ConfigValueView::Id(id) => SessionConfigOptionValue::value_id(id.clone()),
        ConfigValueView::Bool(b) => SessionConfigOptionValue::boolean(*b),
    };
    match connection
        .send_request(SetSessionConfigOptionRequest::new(
            session_id.clone(),
            config_id.clone(),
            protocol_value,
        ))
        .block_task()
        .await
    {
        Ok(resp) => {
            send_config_options_fold(
                mode_tracker,
                config_options_from_protocol(&resp.config_options),
                event_tx,
            );
        }
        Err(e) => {
            // Both: the `Notice` is what a chat pane already shows, and the
            // typed event is what a consumer that required the change acts
            // on. Emitting only the typed one would silently drop the
            // wording the agent panel puts in front of a user.
            let reason = format!("{e:?}");
            let _ = event_tx.unbounded_send(AcpEvent::Notice(format!(
                "set_config_option({config_id}={value:?}) failed — the session keeps its \
                 current value: {reason}"
            )));
            let _ = event_tx.unbounded_send(AcpEvent::ConfigOptionRejected { config_id, reason });
        }
    }
}

/// Fold a full option set through the mode tracker and emit what the host
/// needs: a `ModeChanged` when the mode state actually moved, then the
/// mode-stripped option set. The single emit site shared by the agent-pushed
/// `ConfigOptionUpdate` notification and the `set_config_option` reply, so both
/// carry identical ordering — mode first, matching the adapter's own ordering
/// guarantee for order-sensitive consumers.
fn send_config_options_fold(
    mode_tracker: &ModeTracker,
    options: Vec<ConfigOptionView>,
    event_tx: &UnboundedSender<AcpEvent>,
) {
    let fold = mode_tracker.fold_config_options(options);
    if let Some(state) = fold.mode {
        let _ = event_tx.unbounded_send(AcpEvent::ModeChanged { state });
    }
    let _ = event_tx.unbounded_send(AcpEvent::ConfigOptionsChanged(fold.options));
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

/// Read the agent's advertised session capabilities from the `initialize`
/// response into the host view model. `session/load` support is a top-level
/// `AgentCapabilities` bool; the rest are presence of the matching optional
/// sub-capability. Called once at connect to gate host affordances (resume /
/// list / close) without the host touching protocol types.
fn session_capabilities_from_protocol(
    caps: &agent_client_protocol::schema::v1::AgentCapabilities,
) -> SessionCapabilitiesView {
    let session = &caps.session_capabilities;
    SessionCapabilitiesView {
        load: caps.load_session,
        list: session.list.is_some(),
        resume: session.resume.is_some(),
        close: session.close.is_some(),
    }
}

/// Decide whether a requested resume can proceed against the agent's advertised
/// capabilities. Returns the session id to `session/load` when resume was
/// requested *and* the agent supports `session/load`; otherwise `None` (start a
/// fresh session). The second element is an advisory message, `Some` only when a
/// requested resume had to be downgraded to a fresh session because the agent
/// does not advertise load support — surfaced as a [`AcpEvent::Notice`] so the
/// user learns the prior conversation can't be replayed.
fn resolve_resume(
    resume: Option<SessionId>,
    supports_load: bool,
) -> (Option<SessionId>, Option<String>) {
    match resume {
        Some(id) if supports_load => (Some(id), None),
        Some(_) => (
            None,
            Some(
                "resume requested but the agent does not advertise session/load — \
                 starting a fresh session; the prior conversation will not be replayed"
                    .to_string(),
            ),
        ),
        None => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ChatItem;
    use agent_client_protocol::schema::v1::{PermissionOptionId, SessionNotification};

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
    fn client_capabilities_withhold_terminal_output_and_the_terminal_methods() {
        // The adapter gates its shell-output sideband on the literal wire shape
        // `_meta.terminal_output === true`, so assert the serialized JSON, not
        // just the builder call. It stays withheld until a live wire capture
        // confirms the three-notification sequence the parser was written
        // against. `terminal` must stay off too: claiming it promises the
        // `terminal/*` methods this client does not implement.
        let json = serde_json::to_value(client_capabilities()).expect("caps serialize");
        assert_eq!(
            json.get("_meta").and_then(|m| m.get("terminal_output")),
            None,
            "the sideband flag is implemented but deliberately not advertised"
        );
        assert_ne!(
            json.get("terminal"),
            Some(&serde_json::Value::Bool(true)),
            "daruda implements no terminal/* method — advertising it would be a lie"
        );
    }

    /// `auth.terminal` is claimed even though top-level `terminal` is not, and
    /// the two must not be conflated: the first only lets the agent *list*
    /// terminal login methods, the second promises the `terminal/*` RPCs.
    ///
    /// Asserted on the serialized shape because the agent gates on the literal
    /// wire path `clientCapabilities.auth.terminal === true` — a builder call
    /// that landed the flag anywhere else would read as "not advertised" and
    /// silently return an empty `authMethods`, which is exactly the failure
    /// this advertisement exists to end.
    #[test]
    fn client_capabilities_claim_native_subagent_sessions_in_the_shape_the_gate_reads() {
        // Pinned against the adapters' own check: an integer `version` of at
        // least 1 and an array `capabilities` containing the key. A shape that
        // merely looks right is silently ignored, and the symptom is an
        // invisible subagent rather than an error.
        let json = serde_json::to_value(client_capabilities()).expect("caps serialize");
        let air = &json["_meta"]["jetbrains"]["air"];
        assert_eq!(air["version"], serde_json::json!(1));
        assert_eq!(
            air["capabilities"],
            serde_json::json!(["nativeSubagentSessions"])
        );
    }

    #[test]
    fn client_capabilities_claim_terminal_auth_without_claiming_terminal_methods() {
        let json = serde_json::to_value(client_capabilities()).expect("caps serialize");
        assert_eq!(
            json.get("auth").and_then(|a| a.get("terminal")),
            Some(&serde_json::Value::Bool(true)),
            "without this the agent advertises no login methods at all"
        );
        assert_ne!(
            json.get("terminal"),
            Some(&serde_json::Value::Bool(true)),
            "auth.terminal must not drag in the terminal/* promise"
        );
        // The companion flag is read at the ROOT, not under `auth`. Nesting it
        // beside `auth.terminal` reads as correct, serializes fine, and is
        // silently ignored — a live capture caught exactly that.
        assert_eq!(
            json.get("_meta")
                .and_then(|m| m.get(super::TERMINAL_AUTH_META_KEY)),
            Some(&serde_json::Value::Bool(true)),
            "terminal-auth belongs on clientCapabilities._meta, not auth._meta"
        );
        assert_eq!(
            json.get("auth").and_then(|a| a.get("_meta")),
            None,
            "the agent never looks here"
        );
    }

    #[test]
    fn session_capabilities_reads_advertised_flags() {
        use agent_client_protocol::schema::v1::{
            AgentCapabilities, SessionCapabilities, SessionCloseCapabilities,
            SessionResumeCapabilities,
        };
        // Advertise load (top-level bool) + resume + close; leave list/fork off.
        let caps = AgentCapabilities::new()
            .load_session(true)
            .session_capabilities(
                SessionCapabilities::new()
                    .resume(SessionResumeCapabilities::new())
                    .close(SessionCloseCapabilities::new()),
            );
        let v = session_capabilities_from_protocol(&caps);
        assert!(v.load, "load_session bool must map to load");
        assert!(v.resume, "advertised resume must map");
        assert!(v.close, "advertised close must map");
        assert!(!v.list, "unadvertised list must be false");
    }

    #[test]
    fn usage_view_maps_tokens_and_cost() {
        use agent_client_protocol::schema::v1::{Cost, UsageUpdate};
        let u = UsageUpdate::new(53_000, 200_000).cost(Cost::new(0.045, "USD"));
        let v = crate::model::UsageView::from(&u);
        assert_eq!(v.used, 53_000);
        assert_eq!(v.size, 200_000);
        let cost = v.cost.expect("cost must map when present");
        assert_eq!(cost.currency, "USD");
        assert!((cost.amount - 0.045).abs() < f64::EPSILON);
    }

    #[test]
    fn usage_view_without_cost_is_none() {
        use agent_client_protocol::schema::v1::UsageUpdate;
        let v = crate::model::UsageView::from(&UsageUpdate::new(10, 100));
        assert!(v.cost.is_none(), "absent cost must stay None");
    }

    #[test]
    fn resolve_resume_loads_when_supported() {
        let id = SessionId::from("sess-1");
        let (to_load, notice) = resolve_resume(Some(id.clone()), true);
        assert_eq!(to_load, Some(id));
        assert!(notice.is_none(), "supported resume needs no notice");
    }

    #[test]
    fn resolve_resume_downgrades_to_fresh_when_load_unsupported() {
        let (to_load, notice) = resolve_resume(Some(SessionId::from("sess-1")), false);
        assert!(to_load.is_none(), "must start fresh when load unsupported");
        assert!(notice.is_some(), "downgrade must advise the user");
    }

    #[test]
    fn resolve_resume_fresh_session_is_silent() {
        let (to_load, notice) = resolve_resume(None, true);
        assert!(to_load.is_none());
        assert!(notice.is_none(), "a plain fresh session is not a downgrade");
    }

    #[test]
    fn session_capabilities_default_agent_advertises_nothing() {
        use agent_client_protocol::schema::v1::AgentCapabilities;
        let v = session_capabilities_from_protocol(&AgentCapabilities::new());
        assert_eq!(v, SessionCapabilitiesView::default());
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
        assert!(matches!(
            &views[0].kind,
            crate::model::ConfigOptionKindView::Select { current_value, .. }
                if current_value == "sonnet"
        ));
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

    #[test]
    fn with_connect_timeout_passes_through_a_future_that_resolves_first() {
        let result = smol::block_on(with_connect_timeout(
            "probe",
            Duration::from_secs(5),
            std::future::ready(Ok::<_, agent_client_protocol::Error>(42)),
        ));
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn with_connect_timeout_errors_out_a_hung_future() {
        // A future that never resolves models a stuck handshake (e.g. an
        // SSH-wrapped adapter blocked on a silent auth prompt): without the
        // race against the timer, `run_connection` would await this forever
        // and the host would never see an `AcpEvent`, matching the reported
        // "stuck on Connecting" symptom.
        let result = smol::block_on(with_connect_timeout(
            "probe",
            Duration::from_millis(20),
            std::future::pending::<Result<u32, agent_client_protocol::Error>>(),
        ));
        let err = result.expect_err("a never-resolving future must time out");
        assert!(err.message.contains("probe"), "{}", err.message);
        assert!(err.message.contains("timed out"), "{}", err.message);
    }

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
            .unbounded_send(Command::Prompt("queued during connect".to_string()))
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
                None,
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
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new("mid-permission update")),
                            )),
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
                    for (session, update) in
                        native_subagent_replay(root).into_iter().chain([unknown])
                    {
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

    /// A resume replays the whole conversation before the session is
    /// `Connected`, so the router has to hold the parent/child relation across
    /// that boundary. Nothing else covers it: the capture-replay guard
    /// (`wire_log::replay`) reconstructs from a file rather than a live load,
    /// and the live subagent test above runs entirely after `Connected`.
    #[test]
    fn a_resume_replays_a_subagent_before_the_session_is_connected() {
        let (command_tx, command_rx) = unbounded::<Command>();
        let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
        let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));

        smol::block_on(async move {
            let connection = smol::spawn(run_connection(
                native_subagent_resume_agent(),
                PathBuf::from("."),
                None,
                Vec::new(),
                None,
                Some(SessionId::from("sess-root")),
                command_rx,
                event_tx,
                permission_parks,
            ));

            // The loop exits at `Connected`, so everything counted here
            // necessarily arrived while the session was still loading.
            let mut items: Vec<ChatItem> = Vec::new();
            let mut updates_before_connected = 0usize;
            loop {
                match next_event_within(&mut event_rx).await {
                    Some(AcpEvent::Update(update)) => {
                        updates_before_connected += 1;
                        crate::mapping::apply_update(&mut items, &update);
                    }
                    Some(AcpEvent::Connected { .. }) => break,
                    Some(_) => {}
                    None => panic!("connection never reached Connected"),
                }
            }

            assert!(
                updates_before_connected > 0,
                "the replay is supposed to land during the load, not after it"
            );
            let [ChatItem::ToolCall(parent), ChatItem::ToolCall(child)] = &items[..] else {
                panic!("a launch and its one child, got {items:?}");
            };
            assert!(parent.is_subagent_launch());
            assert_eq!(parent.subagent_type(), Some("Lorentz"));
            assert_eq!(parent.status, crate::model::ToolStatusView::Completed);
            assert_eq!(
                child.parent_tool_id.as_deref(),
                Some(parent.id.as_str()),
                "a resumed child renders inside its launch, not as a top-level row"
            );

            drop(command_tx);
            let _ = connection.await;
        });
    }

    /// The whole native subagent path over a real SDK connection: the compat
    /// notification really does bind to `session/update`, the raw payload
    /// survives, and the router turns a child session's work into the flat
    /// parent/child tool calls the render model already draws.
    #[test]
    fn a_native_subagent_arrives_as_a_launch_card_with_its_child() {
        let (command_tx, command_rx) = unbounded::<Command>();
        let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
        let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));

        smol::block_on(async move {
            let connection = smol::spawn(run_connection(
                native_subagent_agent(),
                PathBuf::from("."),
                None,
                Vec::new(),
                None,
                None,
                command_rx,
                event_tx,
                permission_parks,
            ));

            loop {
                match next_event_within(&mut event_rx).await {
                    Some(AcpEvent::Connected { .. }) => break,
                    Some(_) => {}
                    None => panic!("connection never reached Connected"),
                }
            }
            command_tx
                .unbounded_send(Command::Prompt("go".to_string()))
                .unwrap();

            let mut items: Vec<ChatItem> = Vec::new();
            let mut notices = 0usize;
            loop {
                match next_event_within(&mut event_rx).await {
                    Some(AcpEvent::Update(update)) => {
                        crate::mapping::apply_update(&mut items, &update);
                    }
                    Some(AcpEvent::Notice(_)) => notices += 1,
                    Some(AcpEvent::TurnEnded { .. }) => break,
                    Some(_) => {}
                    None => panic!("turn never ended"),
                }
            }

            let [ChatItem::ToolCall(parent), ChatItem::ToolCall(child)] = &items[..] else {
                panic!("a launch and its one child, got {items:?}");
            };
            assert!(parent.is_subagent_launch());
            assert_eq!(parent.subagent_type(), Some("Lorentz"));
            assert_eq!(parent.status, crate::model::ToolStatusView::Completed);
            assert_eq!(
                child.parent_tool_id.as_deref(),
                Some(parent.id.as_str()),
                "the child renders inside its launch, not as a top-level row"
            );
            assert_eq!(notices, 1, "the unknown kind is reported exactly once");

            drop(command_tx);
            let _ = connection.await;
        });
    }

    /// Measures the SDK 2.0 dispatch semantics the permission park relies on:
    /// the handler in `run_connection` awaits the host's decision *inside*
    /// `on_receive_request`, and its comment claims the connection keeps
    /// pumping concurrently. If instead the dispatch loop is held until the
    /// handler returns (as `HandleDispatchFrom`'s "the server will not process
    /// new messages until this handler returns" warns), the update queued
    /// right behind the permission request can only surface *after*
    /// `respond_permission` — streaming would freeze under every permission
    /// prompt. The `#282` ordered-response barrier is not in play here: it
    /// arms only via `on_receiving_result`, which this crate never calls.
    #[test]
    fn a_parked_permission_request_does_not_stall_update_dispatch() {
        let (command_tx, command_rx) = unbounded::<Command>();
        let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
        let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));
        let handle = AcpSessionHandle {
            commands: command_tx,
            permission_parks: permission_parks.clone(),
        };

        smol::block_on(async move {
            let connection = smol::spawn(run_connection(
                permission_then_update_agent(),
                PathBuf::from("."),
                None,
                Vec::new(),
                None,
                None,
                command_rx,
                event_tx,
                permission_parks,
            ));

            loop {
                match next_event_within(&mut event_rx).await {
                    Some(AcpEvent::Connected { .. }) => break,
                    Some(_) => {}
                    None => panic!("connection never reached Connected"),
                }
            }

            handle.send_prompt("run a tool".to_string());

            let permission_id = loop {
                match next_event_within(&mut event_rx).await {
                    Some(AcpEvent::PermissionRequested { id, .. }) => break id,
                    Some(AcpEvent::Update(_)) => {
                        panic!("update outran the permission request sent before it")
                    }
                    Some(_) => {}
                    None => panic!("permission request never surfaced"),
                }
            };

            // THE MEASUREMENT — the permission is still undecided, so this
            // update only arrives if the parked handler leaves the dispatch
            // loop free.
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::Update(_)) => {}
                Some(other) => panic!("expected the queued update, got {other:?}"),
                None => panic!(
                    "dispatch loop is blocked by the parked permission handler: the \
                     session/update queued behind the permission request never surfaced"
                ),
            }

            handle.respond_permission(
                permission_id,
                PermissionDecision::Allow {
                    option_id: "allow".to_string(),
                },
            );

            loop {
                match next_event_within(&mut event_rx).await {
                    Some(AcpEvent::TurnEnded {
                        completed_normally, ..
                    }) => {
                        assert!(completed_normally);
                        break;
                    }
                    Some(_) => {}
                    None => panic!("turn never ended after the permission decision"),
                }
            }

            // Closing the command channel ends `prompt_loop`, and with it the
            // whole connection task.
            drop(handle);
            connection.await.expect("connection task ends cleanly");
        });
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

    #[test]
    fn connected_carries_the_program_the_agent_reported() {
        let (command_tx, command_rx) = unbounded::<Command>();
        let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
        let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));
        let handle = AcpSessionHandle {
            commands: command_tx,
            permission_parks: permission_parks.clone(),
        };

        smol::block_on(async move {
            let connection = smol::spawn(run_connection(
                self_identifying_agent(),
                PathBuf::from("."),
                None,
                Vec::new(),
                None,
                None,
                command_rx,
                event_tx,
                permission_parks,
            ));

            let program = loop {
                match next_event_within(&mut event_rx).await {
                    Some(AcpEvent::Connected { program, .. }) => break program,
                    Some(_) => {}
                    None => panic!("connection never reached Connected"),
                }
            };

            assert_eq!(
                program.as_deref(),
                Some("@agentclientprotocol/claude-agent-acp"),
                "the reported program reaches the host verbatim"
            );

            drop(handle);
            connection.await.expect("connection task ends cleanly");
        });
    }

    /// The ordering this event exists for: a resume replays the conversation as
    /// updates before `Connected` resolves, so the program has to arrive first
    /// or the restored half of a transcript is mapped under the wrong dialect.
    #[test]
    fn the_program_arrives_before_any_update() {
        let (command_tx, command_rx) = unbounded::<Command>();
        let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
        let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));
        let handle = AcpSessionHandle {
            commands: command_tx,
            permission_parks: permission_parks.clone(),
        };

        smol::block_on(async move {
            let connection = smol::spawn(run_connection(
                self_identifying_agent(),
                PathBuf::from("."),
                None,
                Vec::new(),
                None,
                None,
                command_rx,
                event_tx,
                permission_parks,
            ));

            let mut identified_at: Option<usize> = None;
            let connected_at: usize;
            let mut seen = 0usize;
            loop {
                match next_event_within(&mut event_rx).await {
                    Some(AcpEvent::AgentIdentified { program }) => {
                        assert_eq!(
                            program.as_deref(),
                            Some("@agentclientprotocol/claude-agent-acp")
                        );
                        identified_at.get_or_insert(seen);
                    }
                    Some(AcpEvent::Connected { .. }) => {
                        connected_at = seen;
                        break;
                    }
                    Some(_) => {}
                    None => panic!("connection never reached Connected"),
                }
                seen += 1;
            }

            assert!(
                identified_at.is_some_and(|at| at < connected_at),
                "the program must be known before Connected, since a resume's \
                 replayed updates land in between"
            );

            drop(handle);
            connection.await.expect("connection task ends cleanly");
        });
    }

    /// The advertised logins have to reach the host, and reach it classified:
    /// a host comparing id strings at the call site eventually offers the
    /// metered Console login as if it were the free one.
    #[test]
    fn connected_carries_the_agents_advertised_login_methods() {
        let (command_tx, command_rx) = unbounded::<Command>();
        let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
        let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));
        let handle = AcpSessionHandle {
            commands: command_tx,
            permission_parks: permission_parks.clone(),
        };

        smol::block_on(async move {
            let connection = smol::spawn(run_connection(
                login_advertising_agent(),
                PathBuf::from("."),
                None,
                Vec::new(),
                None,
                None,
                command_rx,
                event_tx,
                permission_parks,
            ));

            let login_methods = loop {
                match next_event_within(&mut event_rx).await {
                    Some(AcpEvent::Connected { login_methods, .. }) => break login_methods,
                    Some(_) => {}
                    None => panic!("connection never reached Connected"),
                }
            };

            assert_eq!(
                login_methods.len(),
                2,
                "both advertised logins reach the host"
            );
            assert_eq!(login_methods[0].kind, crate::LoginMethodKind::Subscription);
            assert_eq!(login_methods[1].kind, crate::LoginMethodKind::MeteredApi);
            // The agent resolved the interpreter for us; the host must not
            // re-derive it.
            assert_eq!(
                login_methods[0]
                    .command
                    .as_ref()
                    .expect("the subscription login carries a terminal-auth block")
                    .program,
                "/opt/node/bin/node"
            );
            // The second method omits `_meta` — normal, not an error.
            assert_eq!(login_methods[1].command, None);

            drop(handle);
            connection.await.expect("connection task ends cleanly");
        });
    }
}
