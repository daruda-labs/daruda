//! `Workspace` method wrappers over the MCP modal free functions.
//!
//! Keeps `render.rs` closures one-liners dispatching through `Workspace`
//! rather than calling `super::open_*` free functions directly.

use gpui::{Context, Window};

use crate::agent::mcp::McpScope;
use crate::workspace::Workspace;

impl Workspace {
    pub(in crate::workspace) fn open_add_mcp_server(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::open_add_mcp_server_modal(self, None, window, cx);
    }

    pub(in crate::workspace) fn open_edit_mcp_server(
        &mut self,
        scope: McpScope,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::open_edit_mcp_server_modal(self, scope, name, window, cx);
    }

    pub(in crate::workspace) fn open_delete_mcp_server_confirm(
        &mut self,
        scope: McpScope,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::open_delete_mcp_server_confirm(self, scope, name, window, cx);
    }
}

#[cfg(test)]
mod tests {
    // Behavioral coverage for these wrappers requires a live `Window`
    // and `Context<Workspace>`; the underlying modal-open logic is
    // exercised by integration tests on the modal modules themselves.
}
