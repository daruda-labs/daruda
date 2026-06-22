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
use gpui::{AnyElement, Context, IntoElement, SharedString, div, prelude::*, px};

use crate::agent::mcp::{McpScope, McpServer, McpSnapshot, McpTransport};
use crate::surface::strings;
use crate::ui::Sizable as _;
use crate::ui::{Divider, button_primary};
use crate::workspace::Workspace;
use crate::workspace::layout::Dock;
use crate::workspace::layout::RightDockSnapshot;

/// Render the Tools tab body.
pub(in crate::workspace) fn render(snap: &RightDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    let mcp = &snap.mcp;
    let workspace = snap.workspace.clone();
    let t = theme::current(cx).clone();

    let has_lane = mcp.project_root.is_some();
    crate::workspace::right_dock::right_panel_body()
        .child(header_row(workspace.clone(), t.text_primary))
        .child(scope_section(
            strings::mcp_project(),
            McpScope::Project,
            mcp,
            workspace.clone(),
            has_lane,
            &t,
        ))
        .child(Divider::horizontal())
        .child(scope_section(
            strings::mcp_local(),
            McpScope::Local,
            mcp,
            workspace.clone(),
            has_lane,
            &t,
        ))
        .child(Divider::horizontal())
        .child(scope_section(
            strings::mcp_user(),
            McpScope::User,
            mcp,
            workspace,
            true,
            &t,
        ))
        .into_any_element()
}

fn header_row(workspace: gpui::WeakEntity<Workspace>, title_color: gpui::Hsla) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .py(px(theme::RIGHT_PANEL_HEADER_PAD_Y))
        .child(
            div()
                .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                .text_color(title_color)
                .child(strings::right_panel_tab_tools()),
        )
        .child(new_server_button(workspace))
}

fn new_server_button(workspace: gpui::WeakEntity<Workspace>) -> impl IntoElement {
    button_primary("mcp-new", strings::mcp_new_button())
        .xsmall()
        .on_click(move |_, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.open_add_mcp_server(window, cx));
            }
        })
}

fn scope_section(
    label: impl Into<gpui::SharedString>,
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
            .text_color(t.text_muted)
            .child(label.into()),
    );

    if !enabled {
        return col
            .child(
                div()
                    .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                    .text_color(t.text_subtle)
                    .child(strings::mcp_no_project_hint()),
            )
            .into_any_element();
    }

    if servers.is_empty() {
        let msg = match scope {
            McpScope::Project => strings::mcp_empty_project(),
            McpScope::Local => strings::mcp_empty_local(),
            McpScope::User => strings::mcp_empty_user(),
        };
        col = col.child(
            div()
                .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                .text_color(t.text_subtle)
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

    // Opaque hover surface (one step up the ladder, matching the Skills
    // tab). The actions overlay reuses it as its background so the
    // revealed Edit / Delete fully mask the status text behind them —
    // a translucent fill let that text bleed through and collide with
    // the buttons.
    let row_hover_bg = theme::BG_HOVER;
    let actions_bg = theme::BG_HOVER;

    let indicator_color = if s.disabled {
        t.text_subtle
    } else if s.is_malformed() {
        t.mcp_indicator_malformed
    } else {
        theme::SIGNAL_GREEN
    };

    // Only the noteworthy states carry a text label — the enabled
    // (default) state is conveyed by the green indicator dot and the
    // full-brightness name alone, so a redundant "enabled" word is
    // omitted.
    let status: Option<(String, gpui::Hsla)> = if s.disabled {
        Some((strings::mcp_status_disabled(), t.text_subtle))
    } else if s.is_malformed() {
        Some((strings::mcp_status_malformed(), t.mcp_malformed_badge_text))
    } else {
        None
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
                .id(SharedString::from(format!("mcp-toggle-{}", s.name)))
                .flex_none()
                .w(px(theme::MCP_INDICATOR_SIZE))
                .h(px(theme::MCP_INDICATOR_SIZE))
                .rounded_full()
                .bg(indicator_color)
                .cursor_pointer()
                .on_click(move |_: &gpui::ClickEvent, _window, cx| {
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
                    t.text_subtle
                } else {
                    t.text_primary
                })
                .child(server_name),
        )
        .child(transport_chip(transport_label, t))
        .child(
            // Always present as the flex spacer that fills the row and
            // backs the hover-action overlay; carries text only for the
            // disabled / malformed states.
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                .when_some(status, |d, (label, color)| d.text_color(color).child(label)),
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
            strings::mcp_button_edit(),
            t.text_primary,
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
            strings::mcp_button_delete(),
            theme::ERROR,
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
        .bg(theme::OVERLAY_SELECTED)
        .text_size(px(theme::MCP_BADGE_FONT_SIZE))
        .text_color(t.text_body)
        .child(label)
}

/// Hover-only text action button. Visually distinct from the Skills
/// tab's chip-shaped `[Edit]` / `[×]` (built from `crate::ui::button`):
/// Tools rows already carry a status indicator dot, so a flat text
/// affordance keeps the row line-height tight. The different shape is
/// deliberate, so this stays a local helper instead of a shared one.
fn text_action_button<F>(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    hover_color: gpui::Hsla,
    t: &DarudaTheme,
    on_click: F,
) -> impl IntoElement
where
    F: Fn(&mut gpui::Window, &mut gpui::App) + 'static,
{
    let idle_color = t.text_body;
    div()
        .id(id)
        .flex_none()
        .text_size(px(theme::MCP_BADGE_FONT_SIZE))
        .text_color(idle_color)
        .cursor_pointer()
        .hover(move |s| s.text_color(hover_color))
        .child(label.into())
        .on_click(move |_: &gpui::ClickEvent, window, cx| {
            on_click(window, cx);
        })
}
