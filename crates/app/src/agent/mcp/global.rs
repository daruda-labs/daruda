//! GPUI Global registration for [`McpState`]. The `impl Global` and
//! bootstrap helper live here so `agent/mcp/mod.rs` stays GPUI-free
//! (CLAUDE.md G2 / G7).

use gpui::{App, Global};

use super::McpState;

impl Global for McpState {}

/// Register an empty `McpState` as the GPUI Global. Called from
/// `main.rs::app.run` before any Workspace; the first Workspace's
/// `refresh_mcp_watcher` then populates it. Idempotent so test
/// fixtures building a Workspace directly don't panic on first
/// `cx.update_global`.
pub fn init(cx: &mut App) {
    if !cx.has_global::<McpState>() {
        cx.set_global(McpState::default());
    }
}
