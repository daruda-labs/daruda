//! One-shot ACP connection: runs `initialize` -> `session/new` ->
//! `session/prompt` against the adapter, auto-approving the first permission
//! option offered. Used by this crate's examples; production session
//! handling (multi-turn prompts, inline permission UI, chat render-model
//! mapping) lives in [`crate::session`].
//!
//! The `agent-client-protocol` connection runs on the smol executor (gpui's),
//! not tokio — the crate spawns via `async-process`. Adapter auth
//! (subscription or API key) is the adapter's own responsibility; daruda
//! supplies no credentials — see [`AdapterCommand::default`].

use std::path::PathBuf;
use std::str::FromStr;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    ContentBlock, InitializeRequest, NewSessionRequest, PromptRequest, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome,
    SessionNotification, TextContent,
};
use agent_client_protocol::{AcpAgent, Agent, ConnectionTo, LineDirection};
use futures::channel::mpsc::UnboundedSender;

/// npm package (with `@latest`) for the ACP Claude agent adapter, used by this
/// crate's examples and tests only (via [`AdapterCommand::default`]). The shipped
/// app does **not** launch through this const — its default agent command lives
/// in `daruda_config::AgentDefinition::claude_default()`; the managed-node path is
/// the generic [`crate::node::NodeRuntime::wrap_command`], which wraps whatever
/// command string it is handed. `@latest` keeps the examples/tests on the newest
/// adapter, which advertises model / effort / mode as session config options; see
/// ../CLAUDE.md for the upstream-version policy.
pub(crate) const ADAPTER_NPM_PACKAGE: &str = "@agentclientprotocol/claude-agent-acp@latest";

/// How to launch the ACP agent adapter.
///
/// Either a bash-style command string (`npx -y <pkg>`) or a JSON launch
/// config. Two JSON shapes are accepted: the SDK's own `AcpAgentConfig`
/// (`{"command":..,"args":[..],"env":{..}}`) and the registry `distribution`
/// shape (`{"type":"stdio","command":..,"args":[..],"env":[{"name":..,"value":..}]}`),
/// so this newtype stays forward-compatible with registry-driven discovery.
///
/// `AcpAgent::from_str` itself only parses the former; the registry shape is
/// translated in [`crate::launch_env`], which every launch passes through.
#[derive(Debug, Clone)]
pub struct AdapterCommand(pub String);

impl Default for AdapterCommand {
    fn default() -> Self {
        // Auth (subscription or API key) is the adapter's responsibility —
        // daruda passes no credentials. Assumes `npx` / `node` are on `PATH`
        // (the System runtime case); the managed-node case rewrites this by
        // passing the command string through
        // [`crate::node::NodeRuntime::wrap_command`].
        Self(format!("npx -y {ADAPTER_NPM_PACKAGE}"))
    }
}

/// An agent launch command plus the ambient environment variables that must
/// not reach the spawned adapter.
///
/// The two travel together because a managed account's isolated credentials
/// only win if the inherited auth-override vars (`ANTHROPIC_API_KEY` and
/// friends) are removed — a command handed onward without its strip list
/// silently authenticates as whatever the user exported. Both fields are
/// public and there is no command-only constructor, so a caller has to state
/// the strip list (an empty `Vec` for the unmanaged System account).
///
/// `strip_env` is deliberately *not* folded into `command` as an
/// `/usr/bin/env` prefix: that would hide the `npx` / `node` launcher token
/// from [`crate::node::command_needs_node`] and skip Node.js provisioning
/// entirely. It is applied at final launch assembly instead, at the single
/// site every launch path funnels through —
/// [`crate::launch_env::prepare_adapter_command`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    /// Bash-style command string or JSON stdio config, as
    /// [`AdapterCommand`] describes.
    pub command: String,
    /// Environment variable names to unset for the spawned adapter.
    pub strip_env: Vec<String>,
}

/// Errors surfaced while connecting or driving the protocol exchange.
#[derive(Debug, thiserror::Error)]
pub enum AcpClientError {
    /// The adapter command string could not be parsed into a transport.
    #[error("invalid adapter command: {0}")]
    Command(String),
    /// The protocol exchange failed (handshake, session, or prompt), already
    /// classified so the host can act on it rather than re-parse a string.
    #[error("protocol error: {0}")]
    Protocol(crate::failure::AcpFailure),
    /// Provisioning the Node.js runtime the adapter needs failed. `Display`
    /// forwards the (user-facing) [`crate::node::NodeError`] message verbatim.
    #[error("{0}")]
    Runtime(#[from] crate::node::NodeError),
}

impl AcpClientError {
    /// Collapse into the classified shape the host consumes.
    ///
    /// This is the one place every connect-time failure — a bad command, a
    /// protocol error, a runtime that would not provision — becomes a single
    /// type carrying a remedy, so the host has no residual `String` arm to
    /// reason about. The `Protocol` arm unwraps rather than re-wraps: its
    /// `Display` prefix exists for logs, not for the user-facing message.
    #[must_use]
    pub fn into_failure(self) -> crate::failure::AcpFailure {
        match self {
            Self::Protocol(failure) => failure,
            Self::Runtime(error) => crate::failure::AcpFailure::from_node_error(&error),
            // Rendered through `Display` so the "invalid adapter command:"
            // prefix survives — the bare string has no other context.
            other @ Self::Command(_) => crate::failure::AcpFailure::unclassified(other.to_string()),
        }
    }
}

/// Events emitted by [`run_one_shot`] as the exchange progresses. Production
/// session handling maps protocol traffic into the chat render model instead
/// (see [`crate::session`]).
#[derive(Debug)]
pub enum SpikeEvent {
    /// `initialize` succeeded; carries the agent's self-description.
    Initialized(String),
    /// `session/new` succeeded.
    SessionCreated,
    /// A `session/update` notification arrived mid-turn.
    Notification(String),
    /// The agent asked for tool permission (auto-approved here).
    PermissionRequested(String),
    /// `session/prompt` completed; carries the stop reason.
    PromptCompleted(String),
    /// Raw wire line (stdin/stdout/stderr of the adapter), for diagnostics.
    Wire { direction: String, line: String },
}

/// Run one prompt against the adapter and emit [`SpikeEvent`]s as the exchange
/// progresses. Awaiting this future drives the whole connection; it returns
/// when the prompt turn completes (or the connection fails).
///
/// `events` and every clone are dropped by the time this returns, so a reader
/// draining the matching receiver observes end-of-stream afterwards.
pub async fn run_one_shot(
    command: AdapterCommand,
    cwd: PathBuf,
    prompt: String,
    events: UnboundedSender<SpikeEvent>,
) -> Result<(), AcpClientError> {
    let wire_tx = events.clone();
    let agent = AcpAgent::from_str(&command.0)
        .map_err(|e| AcpClientError::Command(format!("{e:?}")))?
        .with_debug(move |line, direction| {
            let direction = match direction {
                LineDirection::Stdin => "stdin",
                LineDirection::Stdout => "stdout",
                LineDirection::Stderr => "stderr",
            }
            .to_string();
            let _ = wire_tx.unbounded_send(SpikeEvent::Wire {
                direction,
                line: line.to_string(),
            });
        });

    let notif_tx = events.clone();
    let perm_tx = events.clone();

    agent_client_protocol::Client
        .builder()
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                let _ = notif_tx.unbounded_send(SpikeEvent::Notification(format!(
                    "{:?}",
                    notification.update
                )));
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _connection| {
                let _ =
                    perm_tx.unbounded_send(SpikeEvent::PermissionRequested(format!("{request:?}")));
                // Auto-approves the first option; production permission handling
                // routes the user's actual choice (see crate::session).
                match request.options.first().map(|opt| opt.option_id.clone()) {
                    Some(id) => responder.respond(RequestPermissionResponse::new(
                        RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id)),
                    )),
                    None => responder.respond(RequestPermissionResponse::new(
                        RequestPermissionOutcome::Cancelled,
                    )),
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, move |connection: ConnectionTo<Agent>| async move {
            let init = connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            let _ =
                events.unbounded_send(SpikeEvent::Initialized(format!("{:?}", init.agent_info)));

            let new_session = connection
                .send_request(NewSessionRequest::new(cwd))
                .block_task()
                .await?;
            let session_id = new_session.session_id;
            let _ = events.unbounded_send(SpikeEvent::SessionCreated);

            let resp = connection
                .send_request(PromptRequest::new(
                    session_id,
                    vec![ContentBlock::Text(TextContent::new(prompt))],
                ))
                .block_task()
                .await?;
            let _ = events.unbounded_send(SpikeEvent::PromptCompleted(format!(
                "{:?}",
                resp.stop_reason
            )));

            Ok(())
        })
        .await
        .map_err(|e| AcpClientError::Protocol(crate::failure::AcpFailure::classify(&e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_adapter_command_is_claude_agent_acp() {
        assert_eq!(
            AdapterCommand::default().0,
            "npx -y @agentclientprotocol/claude-agent-acp@latest"
        );
    }

    #[test]
    fn default_adapter_command_parses_into_a_transport() {
        // Exercises the real crate parser without spawning a process.
        AcpAgent::from_str(&AdapterCommand::default().0)
            .expect("default adapter command must parse");
    }
}
