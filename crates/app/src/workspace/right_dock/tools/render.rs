//! Tools tab body — renders the project + personal MCP server scopes
//! pulled from `RightDockSnapshot::mcp` (a snapshot of `Workspace::mcp`).
//!
//! Layout:
//! ```text
//! ┌─ Tools ──────────────────────────── [+ Add server] ┐
//! │  Project                                           │
//! │  ● filesystem  [stdio]  enabled                    │
//! │  ○ playwright  [stdio]  disabled                   │
//! │  ⚠ broken     [stdio]  malformed                   │
//! │  Personal                                          │
//! │  ● context7    [http]   enabled                    │
//! └────────────────────────────────────────────────────┘
//! ```
//!
//! All static text comes from `surface::strings::MCP_*`; pixel +
//! colour values from `crate::ui::theme::MCP_*`.

use crate::ui::theme;
use crate::ui::theme::DarudaTheme;
use gpui::{AnyElement, Context, IntoElement, MouseButton, SharedString, div, prelude::*, px};

use crate::agent::mcp::{McpScope, McpServer, McpSnapshot, McpTransport};
use crate::surface::strings;
use crate::ui::Divider;
use crate::workspace::Workspace;
use crate::workspace::layout::Dock;
use crate::workspace::layout::RightDockSnapshot;

/// Render the Tools tab body.
pub(in crate::workspace) fn render(snap: &RightDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    let mcp = &snap.mcp;
    let workspace = snap.workspace.clone();
    let t = theme::current(cx).clone();

    div()
        .flex()
        .flex_col()
        .px(px(theme::RIGHT_PANEL_PAD_X))
        .py(px(theme::RIGHT_PANEL_PAD_Y))
        .gap(px(theme::MCP_SECTION_GAP))
        .child(header_row(workspace.clone(), &t))
        .child(scope_section(
            strings::MCP_PROJECT,
            McpScope::Project,
            mcp,
            workspace.clone(),
            mcp.project_root.is_some(),
            &t,
        ))
        .child(Divider::horizontal())
        .child(scope_section(
            strings::MCP_PERSONAL,
            McpScope::Personal,
            mcp,
            workspace,
            true,
            &t,
        ))
        .into_any_element()
}

fn header_row(workspace: gpui::WeakEntity<Workspace>, t: &DarudaTheme) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(theme::MCP_HEADER_GAP))
        .child(
            div()
                .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                .text_color(t.mcp_section_header_text)
                .child(strings::RIGHT_PANEL_TAB_TOOLS),
        )
        .child(new_server_button(workspace, t))
}

fn new_server_button(workspace: gpui::WeakEntity<Workspace>, t: &DarudaTheme) -> impl IntoElement {
    let badge_bg = t.mcp_transport_badge_bg;
    let badge_text = t.mcp_transport_badge_text;
    let hover_bg = t.mcp_row_hover_bg;
    div()
        .flex()
        .flex_none()
        .px(px(theme::MCP_BADGE_PAD_X))
        .py(px(theme::MCP_BADGE_PAD_Y))
        .rounded(px(theme::MCP_BADGE_RADIUS))
        .bg(badge_bg)
        .text_size(px(theme::MCP_BADGE_FONT_SIZE))
        .text_color(badge_text)
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .child(strings::MCP_NEW_BUTTON)
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.open_add_mcp_server(window, cx));
            }
        })
}

fn scope_section(
    label: &'static str,
    scope: McpScope,
    state: &McpSnapshot,
    workspace: gpui::WeakEntity<Workspace>,
    enabled: bool,
    t: &DarudaTheme,
) -> AnyElement {
    let servers = state.servers(scope);
    let mut col = div().flex().flex_col().gap(px(theme::MCP_ROW_GAP)).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::MCP_HEADER_GAP))
            .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
            .text_color(t.mcp_section_header_text)
            .child(SharedString::from(label.to_string())),
    );

    if !enabled {
        return col
            .child(
                div()
                    .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                    .text_color(t.mcp_empty_text)
                    .child(strings::MCP_NO_PROJECT_HINT),
            )
            .into_any_element();
    }

    if servers.is_empty() {
        let msg = match scope {
            McpScope::Project => strings::MCP_EMPTY_PROJECT,
            McpScope::Personal => strings::MCP_EMPTY_PERSONAL,
        };
        col = col.child(
            div()
                .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                .text_color(t.mcp_empty_text)
                .child(msg),
        );
        return col.into_any_element();
    }

    for s in servers {
        col = col.child(server_row(s, workspace.clone(), t));
    }
    col.into_any_element()
}

fn server_row(
    s: &McpServer,
    workspace: gpui::WeakEntity<Workspace>,
    t: &DarudaTheme,
) -> AnyElement {
    let scope = s.scope;
    let name = s.name.clone();
    let workspace_toggle = workspace.clone();
    let workspace_edit = workspace.clone();
    let workspace_delete = workspace.clone();
    let name_for_toggle = name.clone();
    let name_for_edit = name.clone();
    let name_for_delete = name.clone();

    let row_hover_bg = t.mcp_row_hover_bg;
    let actions_bg = t.mcp_row_hover_bg;

    let indicator_color = if s.disabled {
        t.mcp_indicator_disabled
    } else if s.is_malformed() {
        t.mcp_indicator_malformed
    } else {
        t.mcp_indicator_enabled
    };

    let (status_label, status_color) = if s.disabled {
        (strings::MCP_STATUS_DISABLED, t.mcp_disabled_badge_text)
    } else if s.is_malformed() {
        (strings::MCP_STATUS_MALFORMED, t.mcp_malformed_badge_text)
    } else {
        (strings::MCP_STATUS_ENABLED, t.mcp_row_body_text)
    };

    let server_name = SharedString::from(s.name.clone());
    let transport_label = match s.transport {
        McpTransport::Stdio => strings::MCP_TRANSPORT_STDIO,
        McpTransport::Sse => strings::MCP_TRANSPORT_SSE,
        McpTransport::Http => strings::MCP_TRANSPORT_HTTP,
    };

    div()
        .id(SharedString::from(s.row_dom_id()))
        .group("mcp-row")
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .min_w_0()
        .gap(px(theme::MCP_HEADER_GAP))
        .px(px(theme::RIGHT_PANEL_PAD_X))
        .py(px(theme::SKILL_BADGE_PAD_Y))
        .rounded(px(theme::MCP_BADGE_RADIUS))
        .hover(move |d| d.bg(row_hover_bg))
        .child(
            div()
                .flex_none()
                .w(px(theme::MCP_INDICATOR_SIZE))
                .h(px(theme::MCP_INDICATOR_SIZE))
                .rounded_full()
                .bg(indicator_color)
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    if let Some(ws) = workspace_toggle.upgrade() {
                        let n = name_for_toggle.clone();
                        ws.update(cx, |ws, cx| ws.toggle_mcp_server(scope, &n, cx));
                    }
                }),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                .text_color(if s.disabled {
                    t.mcp_disabled_badge_text
                } else {
                    t.modal_text_primary
                })
                .child(server_name),
        )
        .child(transport_chip(transport_label, t))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                .text_color(status_color)
                .child(status_label),
        )
        .child(
            div()
                .absolute()
                .right(px(theme::RIGHT_PANEL_PAD_X))
                .top_0()
                .bottom_0()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::MCP_HEADER_GAP))
                .bg(actions_bg)
                .pl(px(theme::MCP_HEADER_GAP))
                .invisible()
                .group_hover("mcp-row", |s| s.visible())
                .child(row_actions(
                    scope,
                    name_for_edit,
                    name_for_delete,
                    workspace_edit,
                    workspace_delete,
                    t,
                )),
        )
        .into_any_element()
}

fn row_actions(
    scope: McpScope,
    name_for_edit: String,
    name_for_delete: String,
    workspace_edit: gpui::WeakEntity<Workspace>,
    workspace_delete: gpui::WeakEntity<Workspace>,
    t: &DarudaTheme,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .flex_none()
        .gap(px(theme::MCP_HEADER_GAP))
        .child(text_action_button(
            "edit",
            strings::MCP_BUTTON_EDIT,
            t,
            move |window, cx| {
                if let Some(ws) = workspace_edit.upgrade() {
                    let n = name_for_edit.clone();
                    ws.update(cx, |ws, cx| ws.open_edit_mcp_server(scope, n, window, cx));
                }
            },
        ))
        .child(text_action_button(
            "del",
            strings::MCP_BUTTON_DELETE,
            t,
            move |window, cx| {
                if let Some(ws) = workspace_delete.upgrade() {
                    let n = name_for_delete.clone();
                    ws.update(cx, |ws, cx| {
                        ws.open_delete_mcp_server_confirm(scope, n, window, cx)
                    });
                }
            },
        ))
}

fn transport_chip(label: &'static str, t: &DarudaTheme) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(theme::MCP_BADGE_PAD_X))
        .py(px(theme::MCP_BADGE_PAD_Y))
        .rounded(px(theme::MCP_BADGE_RADIUS))
        .bg(t.mcp_transport_badge_bg)
        .text_size(px(theme::MCP_BADGE_FONT_SIZE))
        .text_color(t.mcp_transport_badge_text)
        .child(label)
}

/// Hover-only text action button. Visually distinct from the Skills
/// tab's chip-shaped `[Edit]` (which has a tinted background) — Tools
/// rows have an indicator dot already, so a flat text affordance keeps
/// the row line-height tight. Different shape on purpose, hence not
/// shared with `skills::render::text_action_button`.
fn text_action_button<F>(
    id: &'static str,
    label: &'static str,
    t: &DarudaTheme,
    on_click: F,
) -> impl IntoElement
where
    F: Fn(&mut gpui::Window, &mut gpui::App) + 'static,
{
    let idle_color = t.mcp_transport_badge_text;
    let hover_color = t.modal_text_primary;
    div()
        .id(id)
        .flex_none()
        .text_size(px(theme::MCP_BADGE_FONT_SIZE))
        .text_color(idle_color)
        .cursor_pointer()
        .hover(move |s| s.text_color(hover_color))
        .child(label)
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            on_click(window, cx);
        })
}
