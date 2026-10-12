//! Compatibility entry points; pane contexts own MCP watcher reconciliation.
use crate::workspace::Workspace;
use gpui::{Context, Window};
impl Workspace {
    pub fn refresh_mcp_watcher(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_library(cx);
    }
    pub(in crate::workspace) fn refresh_mcp_on_cwd_change(&mut self, cx: &mut Context<Self>) {
        self.refresh_library(cx);
    }
}
