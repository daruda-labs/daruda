//! The orchestrator's lifetime.
//!
//! Lazy on purpose: starting one at launch costs an agent process
//! and a slice of the account's rate limit on every day the user never says
//! `/daruda`. The first request pays for it instead.
//!
//! No session is resumed. `session/load` would carry yesterday's context into
//! today's conversation with no way for the user to see what it remembers, so
//! every run gets a fresh session.
//!
//! There is no separate state machine here. Whether the orchestrator is up is
//! exactly whether the registered host owns an orchestrator chat slot,
//! and a second copy of that answer could only disagree with it —
//! so [`pane`] asks the registry and nothing caches the result.

pub(crate) mod briefing;
pub(crate) mod config;
pub(crate) mod window;

use gpui::{App, AppContext as _, Global};

use crate::control::mcp::socket;
use crate::control::result::ControlError;
use crate::telegram::bridge::PaneRef;
use crate::window_registry::WindowRegistry;

/// Why the orchestrator cannot take a prompt. Returned rather than reported:
/// the caller is answering a phone, and each of these needs its own wording
/// there. The internal detail behind `OpenFailed` is also logged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EnsureError {
    /// `[orchestrator] enabled = false`.
    Disabled,
    /// Switched on, but the settings name no runnable agent.
    Unresolvable,
    OpenFailed(String),
}

/// The control socket this run serves, and the token its shim must present.
///
/// A GPUI global rather than a field on the orchestrator's `Workspace`: the
/// socket outlives any one window (closing its host workspace leaves the
/// socket bound, so the next `/daruda` reuses it instead of rebinding), and
/// it is one per process by construction.
struct ControlSurface {
    /// Keeps the socket, its lock and its files alive; dropped with the
    /// global. `None` only in tests, which seed a surface without binding
    /// anything: a real listener registers with smol's reactor, and gpui's
    /// test scheduler panics the moment it sees reactor-thread activity.
    _server: Option<socket::Server>,
    gate: socket::TokenGate,
    runtime_id: String,
}

impl Global for ControlSurface {}

/// Start the control socket if it is not already serving.
///
/// Before the window, always: an agent may spawn its shim the instant its
/// session starts, and a shim that finds no socket reports "daruda is not
/// running" to a daruda that is. A failure here refuses the whole request —
/// an orchestrator with no tools is not worth opening a window for.
fn ensure_control_surface(cx: &mut App) -> Result<(), EnsureError> {
    if cx.has_global::<ControlSurface>() {
        return Ok(());
    }
    serve_control_surface(&control_dir(), cx)
}

/// The directory the socket, its lock and its runtime file live in.
///
/// There is deliberately no answer under `cfg(test)`: a test that got here
/// would contend for the profile's lock with the daruda the developer is
/// running, and unlink that app's socket and runtime file when the fixture
/// drops. Fixtures install a socket-free surface instead — see
/// [`seed_control_surface_for_test`].
fn control_dir() -> std::path::PathBuf {
    #[cfg(test)]
    {
        panic!(
            "a test reached the profile's control socket; call \
             orchestrator::seed_control_surface_for_test first"
        )
    }
    #[cfg(not(test))]
    {
        daruda_store::persistence::default_data_dir()
    }
}

/// A surface that answers every question except the socket's.
///
/// The socket is what a test cannot have (see [`ControlSurface::_server`]), and
/// none of what the rest of the app asks a surface for — the session token, the
/// run's identity — needs one.
#[cfg(test)]
pub(crate) fn seed_control_surface_for_test(cx: &mut App) {
    cx.set_global(ControlSurface {
        _server: None,
        gate: socket::TokenGate::new(),
        runtime_id: socket::new_runtime_id(),
    });
}

/// Bind the socket and publish its runtime file, then keep the inbound queue
/// drained.
fn serve_control_surface(dir: &std::path::Path, cx: &mut App) -> Result<(), EnsureError> {
    let (server, inbound) = socket::Server::start(dir, cx).map_err(|e| {
        window::log_open_failure(&window::OpenError::ControlSurfaceUnavailable(e.to_string()));
        EnsureError::OpenFailed(e.to_string())
    })?;
    let runtime_id = server.runtime_id.clone();
    let gate = server.gate.clone();
    cx.set_global(ControlSurface {
        _server: Some(server),
        gate,
        runtime_id,
    });
    // Frames are answered on the foreground, because a tool call touches app
    // state. One `ConnectionSession` for the whole surface: the socket serves
    // one connection at a time, and a new connection resets it.
    cx.spawn(async move |cx| {
        let mut session = crate::control::mcp::dispatch::ConnectionSession::default();
        while let Ok(message) = inbound.recv().await {
            // No `Result` to handle: `AsyncApp::update` returns the closure's
            // value, and a released app drops this task rather than erroring.
            cx.update(|cx| {
                // Read fresh each time: the orchestrator's pane appears after
                // the socket does, and can go away while it is still served.
                let protected = pane(cx);
                crate::control::mcp::dispatch::answer(message, &mut session, protected, cx)
            });
        }
    })
    .detach();
    Ok(())
}

/// The MCP server this run's orchestrator session should reach.
///
/// `None` when the control surface is not up. Only ever handed to the
/// orchestrator's own session — a lane agent with these tools could drive the
/// app, which is the whole threat model.
///
/// **Mints a token, so calling this is not free of consequence**: the token is
/// bound to the session being created and rotating it retires the previous
/// one, which is what makes a discarded session's shim stop working. Call it
/// once per session, from the one place a session is planned. `&App` rather
/// than `&mut App` because the gate is shared with the accept loop and carries
/// its own lock — the mutation is the gate's, not the global's.
pub(crate) fn mcp_server(cx: &App) -> Option<daruda_acp::McpServer> {
    let surface = cx.try_global::<ControlSurface>()?;
    // The shim is this same binary, so there is nothing to install and no
    // version skew between the app and its relay.
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        // Reported, not shrugged off: the session still opens, and an
        // orchestrator with no tools looks like a model that refuses to act
        // rather than like a daruda that could not name its own binary.
        Err(e) => {
            daruda_store::observability::log_writer::LogWriter::log(
                daruda_store::observability::error_report::ErrorReport::new(
                    "Orchestrator gets no control tools: daruda cannot locate its own binary",
                )
                .severity(daruda_store::observability::error_report::ErrorSeverity::Error)
                .from_error(&e)
                .at(file!(), line!())
                .dedup("orchestrator.mcp.current_exe")
                .build(),
            );
            return None;
        }
    };
    Some(daruda_acp::stdio_mcp_server(
        mcp_server_name(&surface.runtime_id),
        executable,
        vec![crate::control::mcp::shim::SUBCOMMAND.to_owned()],
        // The token travels in the session's environment, never in a file
        // another local process could read.
        vec![(
            daruda_core::process_env::CONTROL_TOKEN.name().to_owned(),
            surface.gate.rotate(),
        )],
    ))
}

/// The status bar chip's click: bring the orchestrator up if it is not, then
/// show or hide its tab.
///
/// Not a `Workspace` method: this runs inside the clicked window's dispatch,
/// so seeding has to go through the `&mut Window` already in hand — see
/// [`prepare`].
///
/// When the resolved host is a *different* window the session still starts
/// there, but the tab is left alone rather than inserted into a window the
/// person did not click.
pub(crate) fn start_or_toggle_from_chip(
    chip_host: &gpui::WeakEntity<crate::workspace::Workspace>,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    match prepare(cx) {
        // Already up — nothing to start, just toggle below.
        Ok(None) => {}
        // The clicked window is mid-update; a different host is not.
        Ok(Some(prepared)) if prepared.handle == window.window_handle() => {
            prepared.seed(window, cx);
        }
        Ok(Some(prepared)) => {
            let handle = prepared.handle;
            if let Err(error) = cx.update_window(handle, |_, host, cx| prepared.seed(host, cx)) {
                WindowRegistry::clear_orchestrator(cx);
                report_chip_start_failure(chip_host, &open_failed(error), cx);
                return;
            }
        }
        Err(error) => {
            report_chip_start_failure(chip_host, &error, cx);
            return;
        }
    }
    let host_is_the_clicked_window = WindowRegistry::orchestrator(cx)
        .is_some_and(|(_, host)| host.entity_id() == chip_host.entity_id());
    if !host_is_the_clicked_window {
        return;
    }
    if let Err(error) = chip_host.update(cx, |ws, cx| ws.toggle_orchestrator_tab(window, cx)) {
        daruda_store::observability::log_writer::LogWriter::log(
            daruda_store::observability::error_report::ErrorReport::new(
                "Orchestrator host closed before toggle",
            )
            .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
            .with_context("error", error.to_string())
            .at(file!(), line!())
            .dedup("orchestrator.toggle")
            .build(),
        );
    }
}

/// Tell the person their click did not start anything — a toast, not just a
/// log line, because the chip would otherwise sit unchanged with no way to
/// learn why.
fn report_chip_start_failure(
    chip_host: &gpui::WeakEntity<crate::workspace::Workspace>,
    error: &EnsureError,
    cx: &mut App,
) {
    let report = daruda_store::observability::error_report::ErrorReport::new(
        crate::surface::strings::orchestrator_start_failed(),
    )
    .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
    .with_context("reason", format!("{error:?}"))
    .at(file!(), line!())
    .dedup("orchestrator.chip.start")
    .build();
    if chip_host
        .update(cx, |ws, cx| ws.report_error(report.clone(), cx))
        .is_err()
    {
        // The window went away with the click in flight — the log is all
        // that is left to tell.
        daruda_store::observability::log_writer::LogWriter::log(report);
    }
}

/// Where a `/daruda` prompt goes, and whether asking had to start the
/// orchestrator to answer.
///
/// Starting one is why this cannot be a plain lookup, and why it lives here
/// rather than in `control`: the command core is below this module and should
/// not be reaching up into it to find a pane.
pub(crate) fn destination(cx: &mut App) -> Result<(PaneRef, bool), ControlError> {
    let connecting = pane(cx).is_none();
    let target = ensure(cx).map_err(|e| match e {
        EnsureError::Disabled => ControlError::OrchestratorDisabled,
        EnsureError::Unresolvable => ControlError::OrchestratorUnresolvable,
        EnsureError::OpenFailed(_) => ControlError::OrchestratorUnavailable,
    })?;
    Ok((target, connecting))
}

pub(crate) fn pane(cx: &App) -> Option<PaneRef> {
    let (_, weak) = WindowRegistry::orchestrator(cx)?;
    weak.upgrade()?.read(cx).orchestrator_chat_pane()
}

/// What this run's orchestrator session is told before its first prompt.
///
/// `None` when the control surface is not up — an agent with no tools has
/// nothing to be briefed about, and the caller refuses the whole request
/// before it gets this far anyway.
pub(crate) fn session_briefing(cx: &App) -> Option<String> {
    let surface = cx.try_global::<ControlSurface>()?;
    Some(briefing::briefing(&mcp_server_name(&surface.runtime_id)))
}

/// The MCP server name this run advertises.
///
/// Suffixed with the runtime id because `codex-acp` silently drops a server
/// whose name already exists in the user's Codex config — a collision would
/// leave the orchestrator with no tools and no error anywhere. A per-run
/// suffix makes that collision impossible.
pub(crate) fn mcp_server_name(runtime_id: &str) -> String {
    let short = &runtime_id[..runtime_id.len().min(MCP_NAME_SUFFIX_LEN)];
    format!("{}-{short}", crate::surface::constants::AGENT_FACING_NAME)
}

/// How much of the runtime id the server name carries. Enough that two runs
/// on one machine cannot collide, short enough to stay readable in an agent's
/// tool listing.
const MCP_NAME_SUFFIX_LEN: usize = 8;

/// Bring the orchestrator up if it is not, and answer where to send a prompt.
///
/// The ACP handshake is *not* waited on: it is bounded by `daruda_acp`'s own
/// connect timeout and its outcome arrives as a session event. The caller gets
/// the pane immediately and the prompt queues behind the connect — which is
/// what the phone's "accepted" reply already means.
pub(crate) fn ensure(cx: &mut App) -> Result<PaneRef, EnsureError> {
    let Some(prepared) = prepare(cx)? else {
        // Already up: `prepare` answers with the live pane.
        return pane(cx).ok_or_else(|| open_failed("host released"));
    };
    let handle = prepared.handle;
    match cx.update_window(handle, |_, window, cx| prepared.seed(window, cx)) {
        Ok(pane) => Ok(pane),
        Err(error) => {
            WindowRegistry::clear_orchestrator(cx);
            Err(open_failed(error))
        }
    }
}

/// Everything about bringing the orchestrator up that needs no window, plus
/// what the seeding step will need. `Ok(None)` when a session is already up.
///
/// Split from [`Prepared::seed`] because a caller holding a live `&mut Window`
/// for the host must seed through *that* window: `cx.update_window` on a
/// window whose update is already in progress answers "window not found"
/// (`crates/app/src/CLAUDE.md`, GPUI Result handling). The status bar chip is
/// exactly that caller.
fn prepare(cx: &mut App) -> Result<Option<Prepared>, EnsureError> {
    // Resolve before reusing a live pane so disabling the feature takes effect
    // on the next request.
    let resolved = config::resolve(cx).ok_or_else(|| refusal(cx))?;
    // Socket first, then the window, then the session — an agent that spawns
    // its shim immediately must find something to connect to.
    ensure_control_surface(cx)?;
    if pane(cx).is_some() {
        return Ok(None);
    }
    let cwd = window::cwd().map_err(|error| {
        window::log_open_failure(&error);
        EnsureError::OpenFailed(error.to_string())
    })?;
    window::install_instructions(&cwd);
    let (handle, weak) = host_workspace(cx)?;
    let workspace = weak.upgrade().ok_or_else(|| open_failed("host released"))?;
    WindowRegistry::register_orchestrator(handle, weak, cx);
    let briefing = session_briefing(cx);
    Ok(Some(Prepared {
        handle,
        workspace,
        agent_id: resolved.agent_id,
        cwd,
        account: resolved.account,
        briefing,
    }))
}

/// A resolved start, waiting only for a window to seed the chat in.
struct Prepared {
    handle: gpui::AnyWindowHandle,
    workspace: gpui::Entity<crate::workspace::Workspace>,
    agent_id: String,
    cwd: std::path::PathBuf,
    account: daruda_store::accounts::AccountSelection,
    briefing: Option<String>,
}

impl Prepared {
    /// Seed the chat. `window` must be the host's — see [`prepare`].
    fn seed(self, window: &mut gpui::Window, cx: &mut App) -> PaneRef {
        self.workspace.update(cx, |ws, cx| {
            let pane = ws.seed_orchestrator_chat_pane(
                self.agent_id,
                self.cwd,
                self.account,
                self.briefing,
                window,
                cx,
            );
            PaneRef {
                workspace: ws.uuid(),
                pane,
            }
        })
    }
}

/// Reuse the active workspace, or another live workspace when Settings has focus.
fn host_workspace(
    cx: &mut App,
) -> Result<
    (
        gpui::AnyWindowHandle,
        gpui::WeakEntity<crate::workspace::Workspace>,
    ),
    EnsureError,
> {
    host_workspace_with(cx, |cx| {
        let config = crate::settings_store::SettingsStore::global(cx).user_arc();
        let options = orchestrator_host_window_options(&config);
        crate::windows::try_open_workspace_window(config, None, None, options, cx)
            .map_err(open_failed)
    })
}

/// A phone command may create the first workspace, but must not interrupt the
/// window the person is currently using (for example Settings or Welcome).
fn orchestrator_host_window_options(config: &daruda_config::Config) -> gpui::WindowOptions {
    let mut options = crate::windows::build_window_options(config);
    options.focus = false;
    options
}

fn host_workspace_with(
    cx: &mut App,
    open: impl FnOnce(&mut App) -> Result<gpui::AnyWindowHandle, EnsureError>,
) -> Result<
    (
        gpui::AnyWindowHandle,
        gpui::WeakEntity<crate::workspace::Workspace>,
    ),
    EnsureError,
> {
    if let Some(host) =
        WindowRegistry::active_workspace(cx).or_else(|| WindowRegistry::first_workspace(cx))
    {
        return Ok(host);
    }
    let handle = open(cx)?;
    let weak = WindowRegistry::workspace_for_window(handle, cx)
        .ok_or_else(|| open_failed("workspace was not registered"))?;
    Ok((handle, weak))
}

fn open_failed(error: impl std::fmt::Display) -> EnsureError {
    let reason = error.to_string();
    daruda_store::observability::log_writer::LogWriter::log(
        daruda_store::observability::error_report::ErrorReport::new(
            "Orchestrator host unavailable",
        )
        .severity(daruda_store::observability::error_report::ErrorSeverity::Error)
        .with_context("reason", reason.clone())
        .at(file!(), line!())
        .dedup("orchestrator.host")
        .build(),
    );
    EnsureError::OpenFailed(reason)
}

/// Which refusal an unresolvable configuration is. Separate from [`ensure`] so
/// the two reasons the phone must distinguish — "you never turned this on" and
/// "you turned it on but it names no agent" — are decided in one place.
fn refusal(cx: &App) -> EnsureError {
    let config = crate::settings_store::SettingsStore::global(cx).user_arc();
    if config.orchestrator.enabled {
        EnsureError::Unresolvable
    } else {
        EnsureError::Disabled
    }
}

#[cfg(test)]
mod tests;
