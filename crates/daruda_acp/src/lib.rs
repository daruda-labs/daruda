//! ACP (Agent Client Protocol) client core for daruda.
//!
//! Spawns an ACP agent adapter (e.g. `@agentclientprotocol/claude-agent-acp`) as a
//! subprocess and drives the client side of the protocol over JSON-RPC/stdio.
//! GPUI-free: the GPUI view layer in `app` consumes the event stream this
//! crate emits.
//!
//! Task-1 status: the [`connection`] module is the one-shot spike that proves
//! the protocol round-trip and the smol-executor bridge. The long-lived
//! multi-turn connection and the chat render-model mapping build on it.

pub mod adapter;
pub mod connection;
pub mod mapping;
pub mod model;
pub mod node;
pub(crate) mod output_highlight;
pub mod session;

pub use agent_client_protocol::schema::v1::SessionId;
pub use connection::{AcpClientError, AdapterCommand, SpikeEvent, run_one_shot};
pub use mapping::{
    SubagentActivity, UpdateEffect, apply_update, apply_update_with, cancel_pending_tools,
    finalize_streaming, permission_item, subagent_activity, touched_tool_id,
};
pub use model::{
    ChatItem, ConfigChoiceView, ConfigOptionCategoryView, ConfigOptionView, CostView, DiffView,
    ModeStateView, PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution,
    PlanEntryView, PlanPriority, PlanStatus, SessionCapabilitiesView, SessionModeView,
    SlashCommand, SlashCommandInput, ToolCallItem, ToolKindView, ToolOutputBlock, ToolStatusView,
    UsageView,
};
pub use node::{NodeError, NodeProgress, NodeRuntime, ensure_node};
pub use session::{
    AcpEvent, AcpSessionHandle, ConnectPhase, InfoFieldChange, PermissionDecision,
    connect_agent_session, connect_session,
};
