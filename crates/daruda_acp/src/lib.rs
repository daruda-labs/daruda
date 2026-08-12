//! ACP (Agent Client Protocol) client core for daruda.
//!
//! Spawns an ACP agent adapter (e.g. `@agentclientprotocol/claude-agent-acp`) as a
//! subprocess and drives the client side of the protocol over JSON-RPC/stdio.
//! GPUI-free: the GPUI view layer in `app` consumes the event stream this
//! crate emits.
//!
//! [`connection`] runs a one-shot protocol round-trip over the smol-executor
//! bridge; [`session`] builds the long-lived multi-turn connection on top of
//! it, and [`mapping`] turns protocol updates into the chat render model.

pub mod adapter;
pub mod connection;
pub mod launch_env;
pub mod mapping;
pub(crate) mod mode_tracker;
pub mod model;
pub mod node;
pub(crate) mod output_highlight;
pub mod session;
pub(crate) mod wire_log;

pub use agent_client_protocol::schema::v1::SessionId;
// `AcpEvent::PermissionRequested` already hands a protocol
// `RequestPermissionRequest` to its consumer; these are what reading its
// `options` faithfully takes. The kind is re-exported rather than mapped to
// `PermissionKindView` because that mapping renders an unrecognized kind as a
// reject, which is right for a UI and wrong for anything selecting on it.
pub use agent_client_protocol::schema::v1::{
    ContentBlock, PermissionOption, PermissionOptionKind, SessionUpdate,
};
pub use connection::{AcpClientError, AdapterCommand, LaunchSpec, SpikeEvent, run_one_shot};
pub use mapping::{
    SubagentActivity, UpdateEffect, apply_update, apply_update_with, cancel_pending_tools,
    finalize_streaming, permission_item, subagent_activity, touched_tool_id,
};
pub use model::{
    ChatItem, CommandExit, ConfigChoiceView, ConfigOptionCategoryView, ConfigOptionKindView,
    ConfigOptionView, ConfigValueView, CostView, DiffView, ModeStateView, PermissionChoice,
    PermissionItem, PermissionKindView, PermissionResolution, PlanEntryView, PlanPriority,
    PlanStatus, SessionCapabilitiesView, SessionModeView, SlashCommand, SlashCommandInput,
    ToolCallItem, ToolKindView, ToolOutputBlock, ToolStatusView, UsageView,
};
pub use node::{NodeError, NodeProgress, NodeRuntime, ensure_node};
pub use session::{
    AcpEvent, AcpSessionHandle, ConnectPhase, InfoFieldChange, PermissionDecision,
    connect_agent_session, connect_session,
};
