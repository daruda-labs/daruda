//! ACP client connection — spike.
//!
//! Proves the four Task-1 unknowns end to end:
//! 1. the `agent-client-protocol` connection runs on the smol executor
//!    (gpui's executor) — the crate spawns via `async-process`, not tokio;
//! 2. adapter auth is handled by the adapter itself (daruda supplies no
//!    credentials) — see [`AdapterCommand::default`];
//! 3. a full `initialize` -> `session/new` -> `session/prompt` round-trip;
//! 4. a `session/request_permission` round-trip (auto-approved here; the real
//!    inline-card UI is a later task).
//!
//! This is the one-shot seed of the future long-lived `AcpConnection`. The
//! multi-turn prompt loop and the chat render-model mapping are deliberately
//! out of scope for the spike.

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

/// npm package (with `@latest`) for the ACP Claude agent adapter. Single source
/// for both the default `npx` command and the managed-node command in
/// [`crate::node`]. `@latest` keeps us on the newest adapter, which advertises
/// model / effort / mode as session config options; see ../CLAUDE.md for the
/// upstream-version policy.
pub(crate) const ADAPTER_NPM_PACKAGE: &str = "@agentclientprotocol/claude-agent-acp@latest";

/// How to launch the ACP agent adapter.
///
/// `AcpAgent::from_str` accepts either a bash-style command string
/// (`npx -y <pkg>`) or a JSON stdio config matching the registry
/// `distribution` shape (`{"type":"stdio","command":..,"args":[..],"env":[..]}`),
/// so this newtype stays forward-compatible with registry-driven discovery.
#[derive(Debug, Clone)]
pub struct AdapterCommand(pub String);

impl Default for AdapterCommand {
    fn default() -> Self {
        // Auth (subscription or API key) is the adapter's responsibility —
        // daruda passes no credentials. Assumes `npx` / `node` are on `PATH`
        // (the System runtime case); the managed-node case rewrites this via
        // [`crate::node::NodeRuntime::adapter_command`].
        Self(format!("npx -y {ADAPTER_NPM_PACKAGE}"))
    }
}

/// Errors surfaced by the spike connection.
#[derive(Debug, thiserror::Error)]
pub enum AcpClientError {
    /// The adapter command string could not be parsed into a transport.
    #[error("invalid adapter command: {0}")]
    Command(String),
    /// The protocol exchange failed (handshake, session, or prompt).
    #[error("protocol error: {0}")]
    Protocol(String),
    /// Provisioning the Node.js runtime the adapter needs failed. `Display`
    /// forwards the (user-facing) [`crate::node::NodeError`] message verbatim.
    #[error("{0}")]
    Runtime(#[from] crate::node::NodeError),
}

/// Minimal observable surface that proves the round-trip. The real connection
/// (later task) maps protocol traffic into the chat render model instead.
#[derive(Debug)]
pub enum SpikeEvent {
    /// `initialize` succeeded; carries the agent's self-description.
    Initialized(String),
    /// `session/new` succeeded.
    SessionCreated,
    /// A `session/update` notification arrived mid-turn.
    Notification(String),
    /// The agent asked for tool permission (auto-approved by the spike).
    PermissionRequested(String),
    /// `session/prompt` completed; carries the stop reason.
    PromptCompleted(String),
    /// Raw wire line (stdin/stdout/stderr of the adapter) — spike diagnostics.
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
                // Spike: auto-approve the first option. The inline allow/reject
                // card that routes the user's real choice back is a later task.
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
        .map_err(|e| AcpClientError::Protocol(format!("{e:?}")))?;

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
