//! `Workspace` method wrappers over the MCP modal free functions.
//!
//! Keeps `render.rs` closures one-liners dispatching through `Workspace`
//! rather than calling `super::open_*` free functions directly.

use gpui::{Context, Window};

use crate::agent::configuration::library::Target;
use crate::agent::mcp::McpScope;
use crate::workspace::Workspace;

impl Workspace {
    pub(super) fn open_add_mcp_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        super::open_add_mcp_server_modal(self, None, window, cx);
    }

    pub(super) fn open_add_mcp_server_in_target(
        &mut self,
        target: crate::agent::configuration::library::Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::add_modal::open_add_mcp_server_in_target(self, target, window, cx);
    }

    /// `target` is the file the row came from; the modal saves back to it.
    pub(super) fn open_edit_mcp_server(
        &mut self,
        scope: McpScope,
        name: String,
        target: Option<Target>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.right_views.library.selected_target = target;
        super::open_edit_mcp_server_modal(self, scope, name, window, cx);
    }

    pub(super) fn open_delete_mcp_server_confirm(
        &mut self,
        scope: McpScope,
        name: String,
        target: Option<Target>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.right_views.library.selected_target = target;
        super::open_delete_mcp_server_confirm(self, scope, name, window, cx);
    }

    /// Flip a listed server's switch: in its own source file when the row
    /// came from the library, otherwise through the scope's configuration.
    pub(super) fn toggle_listed_mcp_server(
        &mut self,
        scope: McpScope,
        name: &str,
        target: Option<&Target>,
        expected: &crate::agent::mcp::McpServerDraft,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = target else {
            self.toggle_mcp_server(scope, name, cx);
            return;
        };
        if let Err(error) = target.toggle(expected) {
            self.report_error(
                daruda_store::observability::error_report::ErrorReport::new(
                    crate::surface::strings::error::mcp_toggle_failed(
                        crate::surface::strings::mcp::scope_display(scope),
                    ),
                )
                .from_error(&error)
                .at(file!(), line!())
                .build(),
                cx,
            );
        }
        self.refresh_library(cx);
    }
}

#[cfg(test)]
mod tests {
    // Behavioral coverage for these wrappers requires a live `Window`
    // and `Context<Workspace>`; the underlying modal-open logic is
    // exercised by integration tests on the modal modules themselves.
}
