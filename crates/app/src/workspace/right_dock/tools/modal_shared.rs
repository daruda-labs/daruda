//! Helpers shared by the AddMcpServerModal / EditMcpServerModal.

use crate::ui::theme;
use gpui::{IntoElement, SharedString, div, prelude::*, px};

use crate::agent::mcp::{FieldError, McpScope, McpTransport};
use crate::surface::strings;

/// Split a space-separated string into args. Quoted segments are NOT
/// supported in v1 — the modal hint mentions space-separated entry.
pub(super) fn split_args(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

pub(super) fn join_args(args: &[String]) -> String {
    args.join(" ")
}

pub(super) fn transport_options() -> Vec<(McpTransport, &'static str)> {
    vec![
        (McpTransport::Stdio, strings::MCP_TRANSPORT_STDIO),
        (McpTransport::Sse, strings::MCP_TRANSPORT_SSE),
        (McpTransport::Http, strings::MCP_TRANSPORT_HTTP),
    ]
}

pub(super) fn scope_options() -> Vec<(McpScope, String)> {
    vec![
        (McpScope::Project, strings::mcp_scope_project()),
        (McpScope::Local, strings::mcp_scope_local()),
        (McpScope::User, strings::mcp_scope_user()),
    ]
}

/// Small label rendered above a form field in the modal stack-style
/// layout (label-on-top instead of inline).
pub(super) fn field_label(
    text: impl Into<SharedString>,
    t: &crate::ui::theme::DarudaTheme,
) -> impl IntoElement {
    div()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(t.text_muted)
        .child(text.into())
}

/// Map a typed [`FieldError`] back to its localised banner string.
pub(super) fn field_error_to_msg(e: FieldError) -> SharedString {
    match e {
        FieldError::CommandRequired => strings::mcp_command_required().into(),
        FieldError::UrlRequired => strings::mcp_url_required().into(),
        FieldError::UrlInvalidScheme => strings::mcp_url_invalid().into(),
        FieldError::EnvInvalidLine { .. } => strings::mcp_env_invalid().into(),
    }
}
