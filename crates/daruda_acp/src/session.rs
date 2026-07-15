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
use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

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
use futures::future::Either;

use crate::connection::{AcpClientError, AdapterCommand};
use crate::model::{ConfigOptionView, ModeStateView, SessionCapabilitiesView};

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
    /// freshly created session.
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
    TurnFailed(String),
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
/// `stderr` — is written there with a millisecond timestamp and a direction
/// marker, so tool-call / subagent traffic can be inspected off-line. The file
/// is truncated on the first open of each process run and appended to by every
/// later session in the same run, so a restart starts fresh instead of growing
/// without bound. Identity
/// (no tap) when the variable is unset or the file can't be opened, so the
/// shipping build never touches the wire unless explicitly asked. The app sets
/// the variable in debug builds (see `bootstrap::init_observability`); it can
/// also be set by hand to capture from any build.
///
/// `agent_id` (the catalog id, e.g. `"claude"` / `"codex"`, empty for a caller
/// with no such identity like the crate's own examples) is spliced into the
/// configured file name via [`wire_log_path_for`] so concurrent sessions from
/// different agents land in separate files instead of interleaving in one.
fn attach_wire_log(agent: AcpAgent, agent_id: &str) -> AcpAgent {
    let Some(path) = std::env::var_os("DARUDA_ACP_WIRE_LOG") else {
        return agent;
    };
    let path = wire_log_path_for(Path::new(&path), agent_id);
    // Start fresh each process run, then accumulate. `attach_wire_log` runs per
    // ACP session (new chat pane, agent switch, resume) — not per app launch —
    // so a plain `.truncate(true)` would wipe earlier sessions of the same run.
    // Truncate only the first time this process opens a given path, so a restart
    // starts clean while later sessions in the same run append.
    let truncate = wire_log_first_open(&path);
    // Open once (append) and share the handle with the debug closure; a per-line
    // reopen would thrash under streaming turns.
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(!truncate)
        .truncate(truncate)
        .write(truncate)
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

/// Whether `path` is being opened for the wire tap for the first time in this
/// process. The first opener truncates the file (fresh start per app launch);
/// every later session that reuses the same path appends. Tracked in a
/// process-global set so restart-vs-same-run is decided by process lifetime, not
/// by session count.
fn wire_log_first_open(path: &Path) -> bool {
    static SEEN: std::sync::OnceLock<Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    let mut seen = match SEEN
        .get_or_init(|| Mutex::new(std::collections::HashSet::new()))
        .lock()
    {
        Ok(guard) => guard,
        // A poisoned lock only means a prior holder panicked mid-insert; append
        // (don't truncate) so we never wipe an existing run's log on recovery.
        Err(_) => return false,
    };
    seen.insert(path.to_path_buf())
}

/// Splice `-<agent_id>` into `base`'s file name, before the extension (e.g.
/// `acp-wire.log` + `"claude"` → `acp-wire-claude.log`), so each agent's wire
/// tap lands in its own file instead of every session interleaving into one
/// shared `DARUDA_ACP_WIRE_LOG` file. `agent_id` empty leaves `base`
/// untouched — the crate's own examples have no catalog identity to key on.
/// Non-alphanumeric bytes in `agent_id` (a user-editable catalog field) are
/// mapped to `_` so it can never escape `base`'s directory or break the file
/// name.
fn wire_log_path_for(base: &Path, agent_id: &str) -> PathBuf {
    if agent_id.is_empty() {
        return base.to_path_buf();
    }
    let safe_id: String = agent_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = base
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("acp-wire");
    let file_name = match base.extension().and_then(|s| s.to_str()) {
        Some(ext) => format!("{stem}-{safe_id}.{ext}"),
        None => format!("{stem}-{safe_id}"),
    };
    match base.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(file_name),
        _ => PathBuf::from(file_name),
    }
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
///
/// `agent_id` is the catalog id (e.g. `"claude"` / `"codex"`) used only to key
/// the dev-build wire-tap file — see [`attach_wire_log`]. Pass `""` when the
/// caller has no such identity (the crate's own examples).
pub fn connect_session(
    command: AdapterCommand,
    cwd: PathBuf,
    initial_mode: Option<String>,
    resume: Option<SessionId>,
    agent_id: &str,
) -> Result<(AcpSessionHandle, UnboundedReceiver<AcpEvent>), AcpClientError> {
    let agent = AcpAgent::from_str(&command.0)
        .map_err(|e| AcpClientError::Command(format!("{e:?}")))
        .map(|agent| attach_wire_log(agent, agent_id))?;

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
/// download. `agent_id` is the catalog id (e.g. `"claude"` / `"codex"`) —
/// see [`connect_session`]'s doc comment for what it's used for.
pub fn connect_agent_session(
    command: String,
    node_install_dir: PathBuf,
    cwd: PathBuf,
    initial_mode: Option<String>,
    resume: Option<SessionId>,
    agent_id: &str,
    progress: &mut dyn FnMut(crate::node::NodeProgress),
) -> Result<(AcpSessionHandle, UnboundedReceiver<AcpEvent>), AcpClientError> {
    let adapter = if crate::node::command_needs_node(&command) {
        crate::node::ensure_node(&node_install_dir, progress)?
            .wrap_command(&command, &node_install_dir)
    } else {
        AdapterCommand(command)
    };
    connect_session(adapter, cwd, initial_mode, resume, agent_id)
}

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
                    // Live context-window / cost accounting. Surfaced as a typed
                    // event (like mode / plan / config) rather than raw `Update`
                    // so the host renders a context meter without parsing
                    // protocol types. Distinct from the CLI's cumulative Usage
                    // tab: this is the current context fill.
                    SessionUpdate::UsageUpdate(u) => {
                        AcpEvent::UsageChanged(crate::model::UsageView::from(&u))
                    }
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
            // Each handshake step gets its own deadline rather than one budget
            // for the whole sequence, because `session/load` is structurally
            // different: its response only arrives after the adapter replays
            // the *entire* prior conversation, which can legitimately take far
            // longer than `initialize`/`session/new`/`set_mode`'s near-instant
            // replies without being hung — see `CONNECT_RESUME_LOAD_TIMEOUT`.
            // `prompt_loop` below is deliberately outside every timeout here —
            // a live, quiet session is normal.
            let _ = event_tx.unbounded_send(AcpEvent::ConnectProgress(ConnectPhase::Handshaking));
            let (capabilities, resume, fresh) =
                with_connect_timeout("initialize", CONNECT_HANDSHAKE_TIMEOUT, async {
                    let init = connection
                        .send_request(InitializeRequest::new(ProtocolVersion::V1))
                        .block_task()
                        .await?;
                    let capabilities = session_capabilities_from_protocol(&init.agent_capabilities);

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
                    Ok((capabilities, resume, fresh))
                })
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

            // Apply the configured initial mode on a *fresh* session (including a
            // resume downgraded to session/new), when the adapter advertised it,
            // the requested id is in the available list, and it differs from the
            // session's current mode. Skipped on a real load (preserve the resumed
            // mode) and silently when any condition is false, so a misconfigured
            // or non-advertising adapter is forward-compatible.
            //
            // A set_mode failure is NON-FATAL: the session/new already
            // succeeded and the session is usable. Leave mode_state.current
            // at the adapter's real current mode (the chip will reflect that),
            // emit a Notice so the host can log it, and continue to Connected.
            if fresh && let (Some(id), Some(ref mut mode_state)) = (initial_mode, modes.as_mut()) {
                let available = mode_state.available.iter().any(|m| m.id == id);
                if available && mode_state.current != id {
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
                            mode_state.current = id;
                        }
                        Err(e) => {
                            let _ = event_tx.unbounded_send(AcpEvent::Notice(format!(
                                "set_mode({id}) on connect failed — session is active \
                                 in the adapter's default mode: {e:?}"
                            )));
                        }
                    }
                }
            }

            let _ = event_tx.unbounded_send(AcpEvent::Connected {
                session_id: session_id.to_string(),
                modes,
                config_options,
                capabilities,
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
                    let _ = event_tx.unbounded_send(AcpEvent::TurnFailed(format!("{e}")));
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
    use agent_client_protocol::schema::v1::PermissionOptionId;

    #[test]
    fn wire_log_path_splices_agent_id_before_the_extension() {
        assert_eq!(
            wire_log_path_for(Path::new("/logs/acp-wire.log"), "claude"),
            PathBuf::from("/logs/acp-wire-claude.log")
        );
    }

    #[test]
    fn wire_log_path_handles_no_extension() {
        assert_eq!(
            wire_log_path_for(Path::new("/logs/acp-wire"), "codex"),
            PathBuf::from("/logs/acp-wire-codex")
        );
    }

    #[test]
    fn wire_log_path_leaves_base_untouched_when_agent_id_is_empty() {
        assert_eq!(
            wire_log_path_for(Path::new("/logs/acp-wire.log"), ""),
            PathBuf::from("/logs/acp-wire.log")
        );
    }

    #[test]
    fn wire_log_first_open_truncates_once_then_appends() {
        // First open of a given path in this process truncates (fresh start on
        // restart); every later open of the same path appends (same run keeps
        // accumulating). A different path is independently first-opened.
        let path_a = PathBuf::from("/logs/acp-wire-first-open-test-a.log");
        let path_b = PathBuf::from("/logs/acp-wire-first-open-test-b.log");
        assert!(wire_log_first_open(&path_a), "first open truncates");
        assert!(!wire_log_first_open(&path_a), "second open appends");
        assert!(!wire_log_first_open(&path_a), "third open still appends");
        assert!(
            wire_log_first_open(&path_b),
            "a different path truncates once"
        );
    }

    #[test]
    fn wire_log_path_sanitizes_unsafe_agent_id_characters() {
        // A user-editable catalog id must never let a path separator (or other
        // filesystem-unsafe byte) escape the configured log directory.
        assert_eq!(
            wire_log_path_for(Path::new("/logs/acp-wire.log"), "../evil"),
            PathBuf::from("/logs/acp-wire-___evil.log")
        );
    }

    #[test]
    fn wire_log_path_handles_a_bare_file_name_with_no_directory() {
        assert_eq!(
            wire_log_path_for(Path::new("acp-wire.log"), "claude"),
            PathBuf::from("acp-wire-claude.log")
        );
    }

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
}
