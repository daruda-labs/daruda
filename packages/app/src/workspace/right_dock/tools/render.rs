//! Tools tab body — renders the project + personal MCP server scopes
//! pulled from `RightDockSnapshot::mcp` (a snapshot of `Workspace::mcp`).
//!
//! Layout:
//! ```text
//! ┌─ Tools ──────────────────────────────────────── [+] ┐
//! │  ▾ PROJECT                                        2 │
//! │  ▤ filesystem                                  [on] │
//! │    stdio                                            │
//! │  ▤ playwright                                 [off] │
//! │    stdio                                            │
//! │  ─────────────────────────────────────────────────  │
//! │  ▾ USER                                           1 │
//! ├─────────────────────────────────────────────────────┤
//! │  ▤ 3 MCP servers                                    │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! All static text comes from `surface::strings::MCP_*`; pixel +
//! colour values from `crate::ui::theme::MCP_*`.

use crate::ui::Disableable as _;
use crate::ui::theme;
use crate::ui::theme::DarudaTheme;
use gpui::{AnyElement, Context, IntoElement, SharedString, div, prelude::*, px};

use crate::agent::mcp::{McpScope, McpServer};
use crate::surface::strings;
use crate::ui::SectionHeader;
use crate::workspace::Workspace;
use crate::workspace::layout::Dock;
use crate::workspace::layout::RightDockSnapshot;
use crate::workspace::right_dock::section_view::{
    ScopeSection, SectionFold, empty_scope_action, hover_actions, library_row, panel_footer,
};

/// Render the Tools tab body.
pub(in crate::workspace) fn render(snap: &RightDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    let workspace = snap.workspace.clone();
    let mut col =
        crate::workspace::right_dock::right_panel_body().child(header_row(workspace.clone(), cx));
    for (index, group) in snap.library.inventory.groups.iter().enumerate() {
        let target = group.target.clone();
        let t = theme::current(cx);
        let mut rows = div()
            .id(SharedString::from(target.label()))
            .flex()
            .flex_col();
        for server in &group.servers {
            rows = rows.child(server_row(
                server,
                Some(target.clone()),
                workspace.clone(),
                t,
                cx,
            ));
        }
        if group.servers.is_empty() && target.owner.is_none() {
            let message = match target.scope {
                McpScope::User => strings::mcp::add_user(),
                McpScope::Project => strings::mcp::add_project(),
                McpScope::Local => strings::mcp::add_local(),
            };
            let workspace = workspace.clone();
            let target = target.clone();
            rows = rows.child(
                empty_scope_action(
                    SharedString::from(format!("mcp-empty-{}", target.label())),
                    message,
                    cx,
                )
                .tooltip(target.label())
                .on_click(move |_, window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| {
                            ws.open_add_mcp_server_in_target(target.clone(), window, cx)
                        });
                    }
                }),
            );
        }
        use crate::workspace::right_dock::section::DockSection;
        let section = match target.scope {
            McpScope::User => DockSection::ToolsUser,
            McpScope::Project => DockSection::ToolsProject,
            McpScope::Local => DockSection::ToolsLocal,
        };
        let key = crate::workspace::right_dock::section::SectionKey {
            section,
            source: Some(target.label().into()),
        };
        let fold = SectionFold::toggleable(snap.sections.is_open(key.clone()));
        col = col.child(
            div()
                .id(SharedString::from(format!(
                    "source-group-{}",
                    target.label()
                )))
                .child(
                    ScopeSection {
                        section,
                        label: group_label(&snap.library.inventory.groups, &target).into(),
                        count: (!group.servers.is_empty())
                            .then(|| group.servers.len().to_string().into()),
                        fold,
                        divided: index > 0,
                    }
                    .render_keyed(
                        key,
                        fold.is_open().then(|| rows.into_any_element()),
                        &workspace,
                        cx,
                    ),
                ),
        );
    }
    col.into_any_element()
}

/// The scope name, or the plugin that owns the group. Several files in one
/// scope (nested project configurations) also name their source, or their
/// sections would read identically.
fn group_label(
    groups: &[crate::agent::configuration::library::Group],
    target: &crate::agent::configuration::library::Target,
) -> String {
    if let Some(owner) = &target.owner {
        return owner.clone();
    }
    let scope = strings::mcp::scope_display(target.scope);
    let shared = groups
        .iter()
        .filter(|group| group.target.owner.is_none() && group.target.scope == target.scope)
        .count();
    if shared > 1 {
        strings::mcp::group_label(scope, target.label())
    } else {
        scope
    }
}

/// Servers configured across every scope.
pub(in crate::workspace) fn footer(snap: &RightDockSnapshot, cx: &gpui::App) -> AnyElement {
    let total: usize = snap
        .library
        .inventory
        .groups
        .iter()
        .map(|group| group.servers.len())
        .sum();
    if total == 0 {
        return div().into_any_element();
    }
    panel_footer(
        crate::ui::icons::SERVER,
        strings::mcp::footer_servers(total),
        cx,
    )
}

fn header_row(workspace: gpui::WeakEntity<Workspace>, cx: &gpui::App) -> impl IntoElement {
    SectionHeader::new(strings::dock::right_tab_tools())
        .prominent()
        .truncate_label(true)
        .actions(new_server_button(workspace, cx))
}

fn new_server_button(workspace: gpui::WeakEntity<Workspace>, cx: &gpui::App) -> impl IntoElement {
    crate::ui::button_icon("mcp-new", crate::ui::icons::ADD, cx)
        .tooltip(strings::mcp::new_button())
        .on_click(move |_, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.open_add_mcp_server(window, cx));
            }
        })
}

fn server_row(
    s: &McpServer,
    target: Option<crate::agent::configuration::library::Target>,
    workspace: gpui::WeakEntity<Workspace>,
    t: &DarudaTheme,
    cx: &gpui::App,
) -> AnyElement {
    if let Some(target) = target.as_ref()
        && let Some(owner) = target.owner.as_ref()
    {
        return library_row(
            crate::ui::icons::SERVER,
            s.name.clone(),
            Some(
                div()
                    .child(strings::library::read_only(owner))
                    .into_any_element(),
            ),
            cx,
        )
        .id(SharedString::from(format!(
            "readonly-{}-{}",
            target.label(),
            s.name
        )))
        .tooltip(crate::ui::tooltip::text(SharedString::from(target.label())))
        .px(px(theme::LIST_ROW_PAD_X))
        .py(px(theme::PAD_SM))
        .into_any_element();
    }
    let can_toggle = target.as_ref().is_none_or(|target| {
        target.config.provider == crate::agent::configuration::Provider::Codex
            || target.config.cwd.is_some()
    });
    let toggle_target = target.clone();
    let expected = crate::agent::mcp::McpServerDraft::from_server(s);
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

    let summary = s.is_malformed().then(|| {
        div()
            .text_color(t.mcp_malformed_badge_text)
            .child(strings::mcp::status_malformed())
            .into_any_element()
    });
    let source = target
        .as_ref()
        .map(|target| target.label())
        .unwrap_or_default();
    let tooltip = format!("{}\n{}\n{}", s.name, s.transport.slug(), source);
    let name = div()
        .id(SharedString::from(format!("mcp-name-{}", s.row_dom_id())))
        .debug_selector(|| "mcp-name".into())
        .flex_1()
        .min_w_0()
        .truncate()
        .tooltip(crate::ui::tooltip::text(SharedString::from(tooltip)))
        .when(s.disabled, |d| d.text_color(t.text_subtle))
        .child(SharedString::from(s.name.clone()));

    library_row(crate::ui::icons::SERVER, name, summary, cx)
        .id(SharedString::from(s.row_dom_id()))
        .py(px(theme::PAD_SM))
        .group("mcp-row")
        .relative()
        .px(px(theme::LIST_ROW_PAD_X))
        .rounded(px(theme::MCP_BADGE_RADIUS))
        .hover(move |d| d.bg(row_hover_bg))
        .child(
            div().flex_none().self_center().child(
                crate::ui::switch_compact(
                    SharedString::from(format!("mcp-toggle-{}", s.name)),
                    !s.disabled,
                    cx,
                )
                .disabled(!can_toggle)
                .tooltip(if can_toggle {
                    strings::mcp::toggle_tooltip()
                } else {
                    strings::library::toggle_needs_project()
                })
                .debug_selector(|| "mcp-toggle".into())
                .on_click(move |_, _window, cx| {
                    if let Some(ws) = workspace_toggle.upgrade() {
                        ws.update(cx, |ws, cx| {
                            ws.toggle_listed_mcp_server(
                                scope,
                                &name_for_toggle,
                                toggle_target.as_ref(),
                                &expected,
                                cx,
                            )
                        });
                    }
                }),
            ),
        )
        .child(
            // Sits left of the switch so revealing the actions never hides it.
            hover_actions(
                "mcp-row",
                theme::LIST_ROW_PAD_X + crate::ui::switch::COMPACT_TARGET_WIDTH + theme::GAP_SM,
                cx,
            )
            .child(row_actions(
                target,
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
    target: Option<crate::agent::configuration::library::Target>,
    scope: McpScope,
    name_for_edit: String,
    name_for_delete: String,
    workspace_edit: gpui::WeakEntity<Workspace>,
    workspace_delete: gpui::WeakEntity<Workspace>,
    cx: &gpui::App,
) -> impl IntoElement {
    let edit_target = target.clone();
    div()
        .flex()
        .flex_row()
        .flex_none()
        .gap(px(theme::GAP_SM))
        .child(
            crate::ui::button_icon("edit", crate::ui::icons::EDIT, cx)
                .tooltip(strings::common::btn_edit())
                .debug_selector(|| "mcp-edit".into())
                .on_click(move |_, window, cx| {
                    if let Some(ws) = workspace_edit.upgrade() {
                        let n = name_for_edit.clone();
                        ws.update(cx, |ws, cx| {
                            ws.open_edit_mcp_server(scope, n, edit_target.clone(), window, cx)
                        });
                    }
                }),
        )
        .child(
            crate::ui::button_delete_glyph("del", cx)
                .tooltip(strings::common::btn_delete())
                .debug_selector(|| "mcp-delete".into())
                .on_click(move |_, window, cx| {
                    if let Some(ws) = workspace_delete.upgrade() {
                        let n = name_for_delete.clone();
                        ws.update(cx, |ws, cx| {
                            ws.open_delete_mcp_server_confirm(scope, n, target.clone(), window, cx)
                        });
                    }
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_scope_sections_name_their_source() {
        use crate::agent::configuration::{
            Configuration, Provider,
            library::{Group, Target},
        };
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("repo/nested");
        let config =
            Configuration::resolve(Provider::Claude, Some(temp.path().into()), None, Some(cwd))
                .unwrap();
        let primary = Target::new(&config, McpScope::Project, None).unwrap();
        let ancestor =
            Target::new(&config, McpScope::Project, Some(&temp.path().join("repo"))).unwrap();
        let group = |target: &Target| Group {
            target: target.clone(),
            servers: Vec::new(),
        };
        let scope = strings::mcp::scope_display(McpScope::Project);
        assert_eq!(group_label(&[group(&primary)], &primary), scope);
        let both = [group(&primary), group(&ancestor)];
        assert_ne!(group_label(&both, &primary), group_label(&both, &ancestor));
        assert_eq!(
            group_label(&both, &ancestor),
            strings::mcp::group_label(&scope, ancestor.label())
        );
    }

    #[gpui::test]
    fn hover_actions_fit_the_server_row(cx: &mut gpui::TestAppContext) {
        let server = McpServer {
            name: "filesystem-with-a-long-unbroken-server-name".into(),
            scope: McpScope::User,
            transport: crate::agent::mcp::McpTransport::Stdio,
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
            &["mcp-name"],
            Some("mcp-toggle"),
            move |workspace, cx| server_row(&server, None, workspace, theme::current(cx), cx),
        );
    }

    #[gpui::test]
    fn disabled_short_server_name_keeps_the_text_column(cx: &mut gpui::TestAppContext) {
        let server = McpServer {
            name: "node_repl".into(),
            scope: McpScope::User,
            transport: crate::agent::mcp::McpTransport::Stdio,
            command: Some("test-server".into()),
            args: Vec::new(),
            url: None,
            env: Default::default(),
            headers: Default::default(),
            disabled: true,
            extra: Default::default(),
        };
        crate::workspace::right_dock::row_tests::assert_hover_targets_fit_beside(
            cx,
            &["mcp-edit", "mcp-delete"],
            &["mcp-name"],
            Some("mcp-toggle"),
            move |workspace, cx| server_row(&server, None, workspace, theme::current(cx), cx),
        );
    }
}
