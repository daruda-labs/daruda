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
    let Some(pin) = ws.pin_library() else { return };
    let Some(target) = ws
        .right_views
        .library
        .selected_target
        .take()
        .or_else(|| crate::agent::configuration::library::Target::new(&pin, scope, None))
    else {
        return;
    };
    let Some(expected) = ws
        .right_views
        .library
        .view
        .inventory
        .groups
        .iter()
        .find(|g| g.target == target)
        .and_then(|g| g.servers.iter().find(|s| s.name == name))
        .map(crate::agent::mcp::McpServerDraft::from_server)
    else {
        return;
    };
    let path = target.label();
    let workspace = cx.weak_entity();
    let body = format!(
        "{}\n\n{}",
        strings::mcp::delete_body_prefix(),
        strings::mcp::delete_body_detail(&name, path),
    );

    delete_modal::open(
        strings::mcp::delete_title(),
        body,
        move |_window, app_cx| {
            let _pin = &pin;
            let Some(ws) = workspace.upgrade() else {
                return Err(strings::common::form_target_gone().into());
            };
            {
                let name = name.clone();
                ws.update(app_cx, |ws, cx| {
                    if let Err(e) = target.mutate(&name, Some(&expected), None) {
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
                    ws.refresh_library(cx);
                    Ok(())
                })
            }
        },
        window,
        cx,
    );
}
