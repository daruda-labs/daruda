//! The contract every control adapter shares: the command vocabulary and its
//! parser ([`spec`]), what a command answers with ([`result`]), and the one
//! identity a pane is addressed by ([`PaneRef`]).
//!
//! GPUI-free and transport-free. Dispatch, approval, socket and bot polling,
//! and every human-readable rendering stay with the app's adapters.

pub mod agent_text;
pub mod result;
pub mod spec;

use daruda_store::project::WorkspaceUuid;

/// Identifies one agent-chat pane across the whole process: a
/// workspace (window) uuid plus that workspace's locally-scoped pane
/// id. `PaneId` is only unique within a workspace, so routing an
/// inbound reply needs both halves. Mirrors `LaneRef { project, lane }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PaneRef {
    pub workspace: WorkspaceUuid,
    pub pane: u64,
}
