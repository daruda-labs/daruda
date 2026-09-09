//! The orchestrator's lifetime.
//!
//! Lazy on purpose: starting one at launch costs a window, an agent process
//! and a slice of the account's rate limit on every day the user never says
//! `/daruda`. The first request pays for it instead.
//!
//! No session is resumed. `session/load` would carry yesterday's context into
//! today's conversation with no way for the user to see what it remembers, so
//! every run gets a fresh session.
//!
//! There is no separate state machine here. Whether the orchestrator is up is
//! exactly "does `WindowRegistry`'s orchestrator slot hold a window with a
//! chat pane", and a second copy of that answer could only disagree with it —
//! so [`pane`] asks the registry and nothing caches the result.

pub(crate) mod briefing;
pub(crate) mod config;
pub(crate) mod control;
pub(crate) mod window;

use gpui::{App, Global};

use crate::control::mcp::socket;
use crate::telegram::bridge::PaneRef;
use crate::window_registry::WindowRegistry;

/// Why the orchestrator cannot take a prompt. Returned rather than reported:
/// the caller is answering a phone, and each of these needs its own wording
/// there. The internal detail behind `OpenFailed` is logged by
/// [`window::open`]'s caller before this is handed back.
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
/// socket outlives any one window (a closed orchestrator window leaves the
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
        let mut session = control::ConnectionSession::default();
        while let Ok(message) = inbound.recv().await {
            // No `Result` to handle: `AsyncApp::update` returns the closure's
            // value, and a released app drops this task rather than erroring.
            cx.update(|cx| control::answer(message, &mut session, cx));
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
            crate::control::mcp::shim::TOKEN_ENV.to_owned(),
            surface.gate.rotate(),
        )],
    ))
}

/// Where to send a prompt, if the orchestrator is up. The single "is it
/// running?" question, answered from the registry.
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
    // Resolve before reusing a live pane so disabling the feature takes effect
    // on the next request.
    let resolved = config::resolve(cx).ok_or_else(|| refusal(cx))?;
    // Socket first, then the window, then the session — an agent that spawns
    // its shim immediately must find something to connect to.
    ensure_control_surface(cx)?;
    if let Some(pane) = pane(cx) {
        return Ok(pane);
    }
    match window::open(&resolved, cx) {
        Ok(pane) => Ok(pane),
        Err(error) => {
            window::log_open_failure(&error);
            // Deliberately leaves nothing behind: a transient failure must not
            // wedge the feature, so the next `/daruda` retries from scratch.
            Err(EnsureError::OpenFailed(error.to_string()))
        }
    }
}

/// Which refusal an unresolvable configuration is. Split from [`ensure`] so
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
mod tests {
    use super::*;
    use gpui::{BorrowAppContext as _, TestAppContext};

    /// Install a `SettingsStore` holding `config`. `ensure` reads it before
    /// any `Workspace` exists to install one.
    fn with_config(cx: &mut TestAppContext, config: daruda_config::Config) {
        cx.update(|cx| {
            crate::settings_store::SettingsStore::init(cx);
            cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
                store.set_user_for_testing(config);
            });
        });
    }

    fn enabled_naming(agent: Option<&str>) -> daruda_config::Config {
        daruda_config::Config {
            orchestrator: daruda_config::OrchestratorConfig {
                enabled: true,
                agent_id: agent.map(str::to_owned),
                account_id: None,
            },
            ..daruda_config::Config::default()
        }
    }

    /// The threat model in one assertion: a lane agent must never be handed
    /// daruda's tools. `daruda_chat_new` takes an `agent` argument, so if any
    /// pane could pick up the server, the self-target guard's claim to be the
    /// only remaining loop would stop being true.
    #[gpui::test]
    fn only_the_orchestrators_pane_is_offered_the_control_server(cx: &mut TestAppContext) {
        let user = crate::test_support::workspace_with_agent_chat(cx);
        let orchestrator = crate::test_support::register_test_orchestrator(cx);

        // Driven through `update`, not `read_with`: production asks this from
        // inside `Workspace::update`, and `read_with` does not lease — a test
        // that used it would pass against a double-lease panic.
        assert!(
            offered(&user.workspace, user.pane(), cx).is_empty(),
            "no control surface up yet, so nobody gets a server — a session \
             with a server it cannot reach is worse than one with none"
        );
        cx.update(|cx| assert!(mcp_server(cx).is_none()));

        cx.update(seed_control_surface_for_test);

        // The user's pane still gets nothing.
        assert!(
            offered(&user.workspace, user.pane(), cx).is_empty(),
            "a lane agent must not be handed daruda's tools"
        );

        // The orchestrator's pane gets exactly one, and it is the real thing:
        // the shim subcommand plus the session token. Without asserting those,
        // deleting the whole authentication mechanism would still pass.
        let (_, weak) = cx
            .update(|cx| WindowRegistry::orchestrator(cx))
            .expect("registered");
        let orchestrator_ws = weak.upgrade().expect("live");
        let servers = offered(&orchestrator_ws, orchestrator.pane, cx);
        assert_eq!(servers.len(), 1, "the orchestrator gets the control server");
        let described = serde_json::to_value(&servers[0]).expect("serialize");
        assert!(
            described["name"]
                .as_str()
                .is_some_and(|n| n.starts_with(crate::surface::constants::AGENT_FACING_NAME)),
            "D20 names it per run: {described}"
        );
        assert_eq!(
            described["args"],
            serde_json::json!([crate::control::mcp::shim::SUBCOMMAND])
        );
        let env = described["env"].as_array().expect("env");
        let offered_token = env
            .iter()
            .find(|e| e["name"] == crate::control::mcp::shim::TOKEN_ENV)
            .and_then(|e| e["value"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| panic!("the shim authenticates with a token: {described}"));
        assert!(!offered_token.is_empty());

        // Each session gets its own token and retires the last one —
        // otherwise a shim from a discarded session keeps app-driving tools.
        let again = offered(&orchestrator_ws, orchestrator.pane, cx);
        let reissued = serde_json::to_value(&again[0]).expect("serialize");
        assert_ne!(
            reissued["env"][0]["value"].as_str(),
            Some(offered_token.as_str()),
            "a new session must not reuse the previous session's token"
        );
        // That the retired one stops working is `socket`'s half of the rule —
        // see `a_retired_token_no_longer_authenticates`.
    }

    /// Ask through `Workspace::update`, the way production does.
    fn offered(
        workspace: &gpui::Entity<crate::workspace::Workspace>,
        pane: u64,
        cx: &mut TestAppContext,
    ) -> Vec<daruda_acp::McpServer> {
        workspace.update(cx, |ws, cx| ws.mcp_servers_for_pane_for_test(pane, cx))
    }

    /// A pane id that matches the orchestrator's *number* in a different
    /// window must not inherit its tools — the same window-scoping mistake
    /// the lane handles had.
    #[gpui::test]
    fn a_same_numbered_pane_in_another_window_gets_nothing(cx: &mut TestAppContext) {
        let user = crate::test_support::workspace_with_agent_chat(cx);
        let orchestrator = crate::test_support::register_test_orchestrator(cx);
        cx.update(seed_control_surface_for_test);
        // Ask the *user's* workspace about the orchestrator's pane number.
        assert!(
            offered(&user.workspace, orchestrator.pane, cx).is_empty(),
            "which window this is, is half the identity"
        );
    }

    #[test]
    fn the_server_name_carries_a_runtime_suffix() {
        let name = mcp_server_name("a3f9c2d18b7e4051");
        assert_eq!(name, "daruda-a3f9c2d1");
        assert!(
            !name.contains(char::is_whitespace),
            "codex sanitizes whitespace away, so a name must not need it"
        );
    }

    #[test]
    fn two_runtimes_get_different_names() {
        assert_ne!(
            mcp_server_name("aaaaaaaa1111"),
            mcp_server_name("bbbbbbbb2222")
        );
    }

    /// A short id must not panic on the slice — ids come from elsewhere, and
    /// a name is not worth a crash.
    #[test]
    fn a_short_runtime_id_is_used_whole() {
        assert_eq!(mcp_server_name("abc"), "daruda-abc");
        assert_eq!(mcp_server_name(""), "daruda-");
    }

    #[gpui::test]
    fn nothing_is_up_until_something_brings_it_up(cx: &mut TestAppContext) {
        with_config(cx, daruda_config::Config::default());
        cx.update(|cx| assert!(pane(cx).is_none()));
    }

    #[gpui::test]
    fn switching_the_feature_off_refuses_even_with_one_already_up(cx: &mut TestAppContext) {
        let pane_ref = crate::test_support::register_test_orchestrator(cx);
        // `ensure` wants a surface before it will reuse a live pane, and a
        // test cannot bind the profile's.
        cx.update(seed_control_surface_for_test);
        with_config(cx, enabled_naming(None));
        cx.update(|cx| assert_eq!(ensure(cx), Ok(pane_ref), "reused while enabled"));
        with_config(cx, daruda_config::Config::default());
        cx.update(|cx| {
            assert_eq!(ensure(cx), Err(EnsureError::Disabled));
            assert!(
                pane(cx).is_some(),
                "refusing does not tear the window down; closing it is the user's call"
            );
        });
    }

    #[gpui::test]
    fn a_disabled_orchestrator_refuses_without_opening_a_window(cx: &mut TestAppContext) {
        with_config(cx, daruda_config::Config::default());
        cx.update(|cx| {
            assert_eq!(ensure(cx), Err(EnsureError::Disabled));
            assert!(WindowRegistry::orchestrator(cx).is_none());
        });
    }

    /// Switched on but naming an agent the catalog does not hold: a different
    /// refusal from "off", because the fix is a different one.
    #[gpui::test]
    fn an_unresolvable_agent_is_its_own_refusal(cx: &mut TestAppContext) {
        with_config(cx, enabled_naming(Some("no-such-agent")));
        cx.update(|cx| {
            assert_eq!(ensure(cx), Err(EnsureError::Unresolvable));
            assert!(WindowRegistry::orchestrator(cx).is_none());
        });
    }
}
