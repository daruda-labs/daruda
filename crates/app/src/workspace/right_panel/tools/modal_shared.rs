//! Helpers shared by the AddMcpServerModal / EditMcpServerModal.

use crate::ui::theme;
use gpui::{IntoElement, MouseButton, SharedString, div, prelude::*, px};

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

pub(super) fn scope_options() -> Vec<(McpScope, &'static str)> {
    vec![
        (McpScope::Project, strings::MCP_SCOPE_PROJECT),
        (McpScope::Personal, strings::MCP_SCOPE_PERSONAL),
    ]
}

/// Small label rendered above a form field in the modal stack-style
/// layout (label-on-top instead of inline).
pub(super) fn field_label(text: &'static str, cx: &gpui::App) -> impl IntoElement {
    div()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(theme::current(cx).mcp_section_header_text)
        .child(text)
}

/// Pill button used for both the Scope and Transport chip strips. The
/// shape is identical between AddModal and EditModal — keep it here
/// so a future style tweak edits one site.
pub(super) fn chip_button<F>(
    id: SharedString,
    label: &'static str,
    active: bool,
    cx: &gpui::App,
    on_click: F,
) -> impl IntoElement
where
    F: Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
{
    let t = theme::current(cx);
    let (bg, text_color) = if active {
        (t.mcp_transport_badge_text, t.modal_panel_bg)
    } else {
        (t.mcp_transport_badge_bg, t.mcp_transport_badge_text)
    };
    div()
        .id(id)
        .cursor_pointer()
        .px(px(theme::MCP_BADGE_PAD_X))
        .py(px(theme::MCP_BADGE_PAD_Y))
        .rounded(px(theme::MCP_BADGE_RADIUS))
        .bg(bg)
        .text_size(px(theme::MCP_BADGE_FONT_SIZE))
        .text_color(text_color)
        .child(label)
        .on_mouse_down(MouseButton::Left, move |_, w, cx| {
            on_click(&gpui::ClickEvent::default(), w, cx);
        })
}

/// Map a typed [`FieldError`] back to its localised banner string.
pub(super) fn field_error_to_msg(e: FieldError) -> SharedString {
    match e {
        FieldError::CommandRequired => strings::MCP_COMMAND_REQUIRED.into(),
        FieldError::UrlRequired => strings::MCP_URL_REQUIRED.into(),
        FieldError::UrlInvalidScheme => strings::MCP_URL_INVALID.into(),
        FieldError::EnvInvalidLine { .. } => strings::MCP_ENV_INVALID.into(),
    }
}
