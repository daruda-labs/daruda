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

pub mod connection;
pub mod mapping;
pub mod model;
pub mod session;

pub use connection::{AcpClientError, AdapterCommand, SpikeEvent, run_one_shot};
pub use mapping::{apply_update, cancel_pending_tools, finalize_streaming, permission_item};
pub use model::{
    ChatItem, ConfigChoiceView, ConfigOptionCategoryView, ConfigOptionView, DiffView,
    ModeStateView, PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution,
    PlanEntryView, PlanPriority, PlanStatus, SessionModeView, SlashCommand, SlashCommandInput,
    ToolCallItem, ToolKindView, ToolOutputBlock, ToolStatusView,
};
pub use session::{
    AcpEvent, AcpSessionHandle, InfoFieldChange, PermissionDecision, connect_session,
};
