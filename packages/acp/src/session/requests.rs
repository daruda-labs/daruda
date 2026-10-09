//! Protocol request construction, capabilities, and response normalization.

use super::*;

/// `session/new` with the MCP servers this session should reach.
///
/// The field has always been in the schema; daruda simply never used it. A
/// helper rather than an inline assignment so a test can pin the wiring
/// without standing up a connection.
pub(super) fn build_new_session_request(
    cwd: PathBuf,
    mcp_servers: Vec<McpServer>,
) -> NewSessionRequest {
    let mut request = NewSessionRequest::new(cwd);
    request.mcp_servers = mcp_servers;
    request
}

/// `session/load` with the same MCP servers a fresh session would get.
///
/// A resume re-establishes the whole session, servers included — the agent does
/// not remember the list from the run that created the session id. Omitting it
/// here leaves a resumed session connected but tool-less, with nothing to
/// report because the request itself succeeded.
pub(super) fn build_load_session_request(
    session_id: SessionId,
    cwd: PathBuf,
    mcp_servers: Vec<McpServer>,
) -> LoadSessionRequest {
    let mut request = LoadSessionRequest::new(session_id, cwd);
    request.mcp_servers = mcp_servers;
    request
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
pub(super) fn client_capabilities() -> ClientCapabilities {
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

/// Fold a full option set through the mode tracker and emit what the host
/// needs: a `ModeChanged` when the mode state actually moved, then the
/// mode-stripped option set. The single emit site shared by the agent-pushed
/// `ConfigOptionUpdate` notification and the `set_config_option` reply, so both
/// carry identical ordering — mode first, matching the adapter's own ordering
/// guarantee for order-sensitive consumers.
pub(super) fn send_config_options_fold(
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
pub(super) fn config_options_from_protocol(
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
pub(super) fn session_capabilities_from_protocol(
    caps: &agent_client_protocol::schema::v1::AgentCapabilities,
) -> SessionCapabilitiesView {
    let session = &caps.session_capabilities;
    SessionCapabilitiesView {
        load: caps.load_session,
        list: session.list.is_some(),
        resume: session.resume.is_some(),
        close: session.close.is_some(),
        images: caps.prompt_capabilities.image,
        embedded_context: caps.prompt_capabilities.embedded_context,
    }
}

/// Decide whether a requested resume can proceed against the agent's advertised
/// capabilities. Returns the session id to `session/load` when resume was
/// requested *and* the agent supports `session/load`; otherwise `None` (start a
/// fresh session). The second element is an advisory message, `Some` only when a
/// requested resume had to be downgraded to a fresh session because the agent
/// does not advertise load support — surfaced as a [`AcpEvent::Notice`] so the
/// user learns the prior conversation can't be replayed.
pub(super) fn resolve_resume(
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
