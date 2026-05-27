//! Delete confirmation for an MCP server entry — routes through
//! [`crate::workspace::dialog_helpers::open_confirm_dialog`] so every
//! confirm-style dialog in the app shares one opener.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{Context, Window};

use crate::agent::mcp::McpScope;
use crate::surface::strings;
use crate::ui::dialog::ButtonVariant;
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers::open_confirm_dialog;

pub fn open_delete_mcp_server_confirm(
    ws: &mut Workspace,
    scope: McpScope,
    name: String,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let lane = ws.active_lane_root();
    let snapshot = cx
        .global::<crate::agent::mcp::McpState>()
        .snapshot_for(lane.as_deref());
    let Some(path) = snapshot.path_for(scope).map(std::path::Path::to_path_buf) else {
        return;
    };
    let workspace = cx.weak_entity();
    let body = format!(
        "{}\n\nServer: {}\nFile: {}",
        strings::mcp_delete_body_prefix(),
        name,
        path.display(),
    );

    open_confirm_dialog(
        strings::mcp_delete_title(),
        body,
        strings::mcp_button_delete(),
        ButtonVariant::Danger,
        move |_, _window, app_cx| {
            if let Some(ws) = workspace.upgrade() {
                let name = name.clone();
                ws.update(app_cx, |ws, cx| {
                    if let Err(e) = ws.delete_mcp_server_internal(scope, &name, cx) {
                        let report = ErrorReport::new("MCP delete failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("server", name.clone())
                            .dedup("mcp.delete")
                            .build();
                        ws.report_error(report, cx);
                    }
                });
            }
        },
        window,
        cx,
    );
}
