//! Tools tab body — renders the project + personal MCP server scopes
//! pulled from `RightDockSnapshot::mcp` (a snapshot of `Workspace::mcp`).
//!
//! Layout:
//! ```text
//! ┌─ Tools ──────────────────────────────────────── [+] ┐
//! │  ▾ PROJECT                                        2 │
//! │  ▤ filesystem                                  [on] │
//! │    enabled · stdio                                  │
//! │  ▤ playwright                                 [off] │
//! │    disabled · stdio                                 │
//! │  ─────────────────────────────────────────────────  │
//! │  ▾ USER                                           1 │
//! ├─────────────────────────────────────────────────────┤
//! │  ▤ 3 MCP servers                                    │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! All static text comes from `surface::strings::MCP_*`; pixel +
//! colour values from `crate::ui::theme::MCP_*`.

use crate::ui::theme;
use crate::ui::theme::DarudaTheme;
use gpui::{AnyElement, Context, IntoElement, SharedString, div, prelude::*, px};

use crate::agent::mcp::{McpScope, McpServer, McpSnapshot, McpTransport};
use crate::surface::strings;
use crate::ui::SectionHeader;
use crate::workspace::Workspace;
use crate::workspace::layout::Dock;
use crate::workspace::layout::RightDockSnapshot;
use crate::workspace::right_dock::section::DockSection;
use crate::workspace::right_dock::section_view::{
    ScopeSection, SectionFold, library_row, panel_footer,
};

/// Render the Tools tab body.
pub(in crate::workspace) fn render(snap: &RightDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    let mcp = &snap.mcp;
    let workspace = snap.workspace.clone();
    let has_lane = mcp.project_root.is_some();
    let scopes = [
        (
            DockSection::ToolsProject,
            strings::mcp_project(),
            McpScope::Project,
            has_lane,
        ),
        (
            DockSection::ToolsLocal,
            strings::mcp_local(),
            McpScope::Local,
            has_lane,
        ),
        (
            DockSection::ToolsUser,
            strings::mcp_user(),
            McpScope::User,
            true,
        ),
    ];
    let mut col =
        crate::workspace::right_dock::right_panel_body().child(header_row(workspace.clone(), cx));
    for (ix, (section, label, scope, enabled)) in scopes.into_iter().enumerate() {
        let is_open = snap.sections.is_open(section);
        let body = is_open.then(|| scope_body(scope, mcp, workspace.clone(), enabled, cx));
        col = col.child(
            ScopeSection {
                section,
                label: label.into(),
                count: enabled.then(|| mcp.servers(scope).len().to_string().into()),
                fold: SectionFold::toggleable(is_open),
                divided: ix > 0,
            }
            .render(body, &workspace, cx),
        );
    }
    col.into_any_element()
}

/// Servers configured across every scope.
pub(in crate::workspace) fn footer(snap: &RightDockSnapshot, cx: &gpui::App) -> AnyElement {
    let total: usize = [McpScope::Project, McpScope::Local, McpScope::User]
        .into_iter()
        .map(|scope| snap.mcp.servers(scope).len())
        .sum();
    panel_footer(
        crate::ui::icons::SERVER,
        strings::mcp_footer_servers(total),
        cx,
    )
}

fn header_row(workspace: gpui::WeakEntity<Workspace>, cx: &gpui::App) -> impl IntoElement {
    SectionHeader::new(strings::right_panel_tab_tools())
        .prominent()
        .truncate_label(true)
        .actions(new_server_button(workspace, cx))
}

fn new_server_button(workspace: gpui::WeakEntity<Workspace>, cx: &gpui::App) -> impl IntoElement {
    crate::ui::button_icon("mcp-new", crate::ui::icons::ADD, cx)
        .tooltip(strings::mcp_new_button())
        .on_click(move |_, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.open_add_mcp_server(window, cx));
            }
        })
}

fn scope_body(
    scope: McpScope,
    state: &McpSnapshot,
    workspace: gpui::WeakEntity<Workspace>,
    enabled: bool,
    cx: &gpui::App,
) -> AnyElement {
    let t = theme::current(cx);
    if !enabled {
        return empty_hint(strings::mcp_no_project_hint(), t);
    }
    let servers = state.servers(scope);
    if servers.is_empty() {
        let msg = match scope {
            McpScope::Project => strings::mcp_empty_project(),
            McpScope::Local => strings::mcp_empty_local(),
            McpScope::User => strings::mcp_empty_user(),
        };
        return empty_hint(msg, t);
    }
    servers
        .iter()
        .fold(div().flex().flex_col(), |col, s| {
            col.child(server_row(s, workspace.clone(), t, cx))
        })
        .into_any_element()
}

/// Quiet one-line explanation for a scope with nothing to list.
fn empty_hint(msg: String, t: &DarudaTheme) -> AnyElement {
    div()
        .pb(px(theme::DOCK_SECTION_HEADER_PAD_Y))
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(t.text_subtle)
        .child(msg)
        .into_any_element()
}

fn server_row(
    s: &McpServer,
    workspace: gpui::WeakEntity<Workspace>,
    t: &DarudaTheme,
    cx: &gpui::App,
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
    let row_hover_bg = t.skill_row_hover_bg;
    let actions_bg = t.skill_row_hover_bg;

    // The switch mirrors the config's `disabled` flag; there is no live
    // connection state to show, so the summary line reports config only.
    let (status, status_color) = if s.is_malformed() {
        (strings::mcp_status_malformed(), t.mcp_malformed_badge_text)
    } else if s.disabled {
        (strings::mcp_status_disabled(), t.text_subtle)
    } else {
        (strings::mcp_status_enabled(), t.text_muted)
    };
    let transport_label = match s.transport {
        McpTransport::Stdio => strings::MCP_TRANSPORT_STDIO,
        McpTransport::Sse => strings::MCP_TRANSPORT_SSE,
        McpTransport::Http => strings::MCP_TRANSPORT_HTTP,
    };
    let summary = div()
        .text_color(status_color)
        .child(strings::mcp_row_summary(&status, transport_label));
    let name = div()
        .when(s.disabled, |d| d.text_color(t.text_subtle))
        .child(SharedString::from(s.name.clone()));

    library_row(
        crate::ui::icons::SERVER,
        name,
        Some(summary.into_any_element()),
        cx,
    )
    .id(SharedString::from(s.row_dom_id()))
    .group("mcp-row")
    .relative()
    .px(px(theme::SKILL_ROW_PAD_X))
    .rounded(px(theme::MCP_BADGE_RADIUS))
    .hover(move |d| d.bg(row_hover_bg))
    .child(
        div().flex_none().self_center().child(
            crate::ui::switch_compact(
                SharedString::from(format!("mcp-toggle-{}", s.name)),
                !s.disabled,
                cx,
            )
            .tooltip(strings::mcp_toggle_tooltip())
            .debug_selector(|| "mcp-toggle".into())
            .on_click(move |_, _window, cx| {
                if let Some(ws) = workspace_toggle.upgrade() {
                    let n = name_for_toggle.clone();
                    ws.update(cx, |ws, cx| ws.toggle_mcp_server(scope, &n, cx));
                }
            }),
        ),
    )
    .child(
        // Sits left of the switch so revealing the actions never hides it.
        div()
            .absolute()
            .right(px(theme::MCP_ACTIONS_RIGHT))
            .top_0()
            .bottom_0()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP_SM))
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
                cx,
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
    cx: &gpui::App,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .flex_none()
        .gap(px(theme::GAP_SM))
        .child(
            crate::ui::button_icon("edit", crate::ui::icons::EDIT, cx)
                .tooltip(strings::mcp_button_edit())
                .debug_selector(|| "mcp-edit".into())
                .on_click(move |_, window, cx| {
                    if let Some(ws) = workspace_edit.upgrade() {
                        let n = name_for_edit.clone();
                        ws.update(cx, |ws, cx| ws.open_edit_mcp_server(scope, n, window, cx));
                    }
                }),
        )
        .child(
            crate::ui::button_delete_glyph("del", cx)
                .tooltip(strings::mcp_button_delete())
                .debug_selector(|| "mcp-delete".into())
                .on_click(move |_, window, cx| {
                    if let Some(ws) = workspace_delete.upgrade() {
                        let n = name_for_delete.clone();
                        ws.update(cx, |ws, cx| {
                            ws.open_delete_mcp_server_confirm(scope, n, window, cx)
                        });
                    }
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn hover_actions_fit_the_server_row(cx: &mut gpui::TestAppContext) {
        let server = McpServer {
            name: "filesystem".into(),
            scope: McpScope::User,
            transport: McpTransport::Stdio,
            command: Some("test-server".into()),
            args: Vec::new(),
            url: None,
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
            extra: Default::default(),
        };
        crate::workspace::right_dock::row_tests::assert_hover_targets_fit_beside(
            cx,
            &["mcp-edit", "mcp-delete"],
            Some("mcp-toggle"),
            move |workspace, cx| server_row(&server, workspace, theme::current(cx), cx),
        );
    }
}
