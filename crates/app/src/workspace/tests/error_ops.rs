//! Tests for `Workspace::report_error` and toast-queue integration.
//!
//! Properties under test:
//! - the report lands at the head of `error_history`,
//! - the live toast queue surfaces it,
//! - workspace wrappers can dismiss it,
//! - history caps at its workspace-owned limit.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::TestAppContext;

use super::build_workspace;

#[gpui::test]
async fn report_error_appends_to_history_and_pushes_toast(cx: &mut TestAppContext) {
    let (_window, workspace) = build_workspace(cx);

    workspace.update(cx, |ws, cx| {
        let report = ErrorReport::new("PTY died")
            .severity(ErrorSeverity::Error)
            .message("broken pipe")
            .dedup("pty.writer")
            .build();
        ws.report_error(report, cx);
    });

    workspace.read_with(cx, |ws, cx| {
        let history = ws.error_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].title, "PTY died");

        let toasts: Vec<_> = ws.error_toasts(cx).iter().collect();
        assert_eq!(toasts.len(), 1, "live toast surfaces immediately");
        assert_eq!(toasts[0].report.title, "PTY died");
        assert_eq!(toasts[0].repeat_count, 1);
    });

    let target_id = workspace.read_with(cx, |ws, cx| {
        ws.error_toasts(cx).iter().next().expect("toast present").id
    });

    workspace.update(cx, |ws, cx| {
        ws.dismiss_error_toast(target_id, cx);
    });

    workspace.read_with(cx, |ws, cx| {
        assert!(
            ws.error_toasts(cx).is_empty(),
            "dismiss wrapper clears toast"
        );
    });

    workspace.update(cx, |ws, cx| {
        for i in 0..60 {
            let report = ErrorReport::new(format!("Err {i}"))
                .severity(ErrorSeverity::Warning)
                .build();
            ws.report_error(report, cx);
        }
    });

    workspace.read_with(cx, |ws, _cx| {
        let history = ws.error_history();
        assert_eq!(history.len(), 50, "history capped at 50");
        assert_eq!(history[0].title, "Err 59", "newest first");
        assert_eq!(history[49].title, "Err 10", "oldest 10 dropped");
    });
}

/// Pane spawn is core enough to warrant *both* surfaces (status bar
/// pin + transient toast). Regression-guards the wiring so a future
/// refactor can't quietly drop one of them.
#[gpui::test]
async fn report_pane_error_fills_status_bar_and_toast(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::PaneSpawnError;
    use daruda_terminal::pty::PtyError;

    let (_window, workspace) = build_workspace(cx);
    let (tabs_before, panes_before) = workspace.read_with(cx, |ws, _| {
        (
            ws.active_runtime().tabs.len(),
            ws.active_runtime().panes.len(),
        )
    });

    workspace.update(cx, |ws, cx| {
        ws.report_pane_error(
            "Add tab",
            PaneSpawnError::Pty(PtyError::SpawnShell("synthetic shell failure".into())),
            cx,
        );
    });

    workspace.read_with(cx, |ws, cx| {
        assert!(
            ws.last_error.is_some(),
            "status bar pin should be set on pane spawn failure",
        );
        assert!(
            ws.last_error
                .as_ref()
                .is_some_and(|s| s.contains("Add tab failed")),
            "status bar text should mention the operation context",
        );

        let toasts: Vec<_> = ws.error_toasts(cx).iter().collect();
        assert_eq!(toasts.len(), 1, "toast should fire alongside the pin");
        assert_eq!(toasts[0].report.title, "Pane spawn failed: Add tab");
        assert_eq!(toasts[0].report.severity, ErrorSeverity::Error);
        assert_eq!(toasts[0].report.dedup_key.as_deref(), Some("pane.spawn"));
        assert_eq!(
            ws.active_runtime().tabs.len(),
            tabs_before,
            "tabs should be untouched"
        );
        assert_eq!(
            ws.active_runtime().panes.len(),
            panes_before,
            "panes should be untouched"
        );

        // History records it for the (future) "Show recent errors"
        // command palette entry.
        assert_eq!(ws.error_history().len(), 1);
    });
}
