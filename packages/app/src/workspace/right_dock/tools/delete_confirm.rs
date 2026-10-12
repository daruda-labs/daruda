//! Retryable delete confirmation that retains its target and error on failure.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{Context, SharedString, Window};

use super::super::delete_modal;
use crate::agent::mcp::McpScope;
use crate::surface::strings;
use crate::workspace::Workspace;

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
        .snapshot_for(lane.as_deref(), &ws.mcp_project_dirs);
    let Some(path) = snapshot.path_for(scope).map(std::path::Path::to_path_buf) else {
        return;
    };
    let workspace = cx.weak_entity();
    let body = format!(
        "{}\n\n{}",
        strings::mcp::delete_body_prefix(),
        strings::mcp::delete_body_detail(&name, path.display().to_string()),
    );

    delete_modal::open(
        strings::mcp::delete_title(),
        body,
        move |_window, app_cx| {
            let Some(ws) = workspace.upgrade() else {
                return Err(strings::common::form_target_gone().into());
            };
            {
                let name = name.clone();
                ws.update(app_cx, |ws, cx| {
                    if let Err(e) = ws.delete_mcp_server_internal(scope, lane.clone(), &name, cx) {
                        let message = SharedString::from(e.to_string());
                        let report =
                            ErrorReport::new(crate::surface::strings::error::mcp_delete_failed())
                                .severity(ErrorSeverity::Error)
                                .from_error(&e)
                                .at(file!(), line!())
                                .with_context("server", name.clone())
                                .dedup("mcp.delete")
                                .build();
                        ws.report_error(report, cx);
                        return Err(message);
                    }
                    Ok(())
                })
            }
        },
        window,
        cx,
    );
}
