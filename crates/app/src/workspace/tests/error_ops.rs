//! Tests for `Workspace::report_error` (Step 2 entry point) +
//! toast-queue integration (Step 3).
//!
//! Properties under test:
//! - the report lands at the head of `error_history`,
//! - the live toast queue surfaces it (capacity 3, FIFO eviction),
//! - dedup merges repeats and refreshes the expiry,
//! - the 1 Hz expiry sweep self-terminates when the queue empties.

use std::time::Duration;

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

    workspace.read_with(cx, |ws, _cx| {
        let history = ws.error_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].title, "PTY died");

        let toasts: Vec<_> = ws.error_toasts().iter().collect();
        assert_eq!(toasts.len(), 1, "live toast surfaces immediately");
        assert_eq!(toasts[0].report.title, "PTY died");
        assert_eq!(toasts[0].repeat_count, 1);
    });
}

#[gpui::test]
async fn dedup_key_collapses_repeats_into_one_toast(cx: &mut TestAppContext) {
    let (_window, workspace) = build_workspace(cx);

    workspace.update(cx, |ws, cx| {
        for _ in 0..3 {
            let report = ErrorReport::new("PTY died")
                .severity(ErrorSeverity::Error)
                .message("broken pipe")
                .dedup("pty.writer")
                .build();
            ws.report_error(report, cx);
        }
    });

    workspace.read_with(cx, |ws, _cx| {
        let toasts: Vec<_> = ws.error_toasts().iter().collect();
        assert_eq!(
            toasts.len(),
            1,
            "three pushes with same dedup_key collapse to one toast",
        );
        assert_eq!(toasts[0].repeat_count, 3);

        // History keeps each report individually so the user can scroll
        // back through the full sequence.
        assert_eq!(ws.error_history().len(), 3);
    });
}

#[gpui::test]
async fn capacity_evicts_oldest_when_full(cx: &mut TestAppContext) {
    let (_window, workspace) = build_workspace(cx);

    workspace.update(cx, |ws, cx| {
        for i in 0..5 {
            let report = ErrorReport::new(format!("Err {i}"))
                .severity(ErrorSeverity::Error)
                .dedup(format!("k{i}"))
                .build();
            ws.report_error(report, cx);
        }
    });

    workspace.read_with(cx, |ws, _cx| {
        let titles: Vec<&str> = ws
            .error_toasts()
            .iter()
            .map(|t| t.report.title.as_str())
            .collect();
        assert_eq!(
            titles,
            vec!["Err 2", "Err 3", "Err 4"],
            "queue is capped at 3 and evicts oldest",
        );
        assert_eq!(ws.error_history().len(), 5, "history retains all five");
    });
}

#[gpui::test]
async fn dismiss_error_toast_removes_specific_id(cx: &mut TestAppContext) {
    let (_window, workspace) = build_workspace(cx);

    workspace.update(cx, |ws, cx| {
        for i in 0..3 {
            let report = ErrorReport::new(format!("Err {i}"))
                .severity(ErrorSeverity::Info)
                .dedup(format!("k{i}"))
                .build();
            ws.report_error(report, cx);
        }
    });

    let target_id = workspace.read_with(cx, |ws, _cx| {
        ws.error_toasts()
            .iter()
            .find(|t| t.report.title == "Err 1")
            .expect("Err 1 toast present")
            .id
    });

    workspace.update(cx, |ws, cx| {
        ws.dismiss_error_toast(target_id, cx);
    });

    workspace.read_with(cx, |ws, _cx| {
        let titles: Vec<&str> = ws
            .error_toasts()
            .iter()
            .map(|t| t.report.title.as_str())
            .collect();
        assert_eq!(
            titles,
            vec!["Err 0", "Err 2"],
            "dismissing by stable id removes the right toast",
        );
    });
}

/// Regression: 50-item history cap (separate concern from the
/// 3-item toast queue cap).
#[gpui::test]
async fn error_history_caps_at_fifty(cx: &mut TestAppContext) {
    let (_window, workspace) = build_workspace(cx);

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

#[gpui::test]
async fn info_toast_auto_dismisses_after_severity_window(cx: &mut TestAppContext) {
    let (_window, workspace) = build_workspace(cx);

    workspace.update(cx, |ws, cx| {
        let report = ErrorReport::new("Transient")
            .severity(ErrorSeverity::Info)
            .dedup("flap")
            .build();
        ws.report_error(report, cx);
    });

    workspace.read_with(cx, |ws, _cx| {
        assert_eq!(ws.error_toasts().iter().count(), 1);
    });

    // The expiry sweep ticks every second. Walking the virtual clock
    // forward in 1 s steps lets the sweep fire its `expire_tick` calls;
    // after Info's 5 s window the queue should be empty.
    for _ in 0..7 {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }

    workspace.read_with(cx, |ws, _cx| {
        assert!(
            ws.error_toasts().is_empty(),
            "Info toast should have auto-dismissed after 5 s",
        );
        assert_eq!(
            ws.error_history().len(),
            1,
            "history should not be touched by auto-dismiss",
        );
    });
}

#[gpui::test]
async fn error_severity_keeps_toast_for_longer_window(cx: &mut TestAppContext) {
    let (_window, workspace) = build_workspace(cx);

    workspace.update(cx, |ws, cx| {
        let report = ErrorReport::new("Critical")
            .severity(ErrorSeverity::Error)
            .dedup("crit")
            .build();
        ws.report_error(report, cx);
    });

    // Past Info / Warning windows but well under Error's 30 s.
    for _ in 0..10 {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }

    workspace.read_with(cx, |ws, _cx| {
        assert_eq!(
            ws.error_toasts().iter().count(),
            1,
            "Error toast should still be visible at t = 10 s",
        );
    });

    // Past 30 s.
    for _ in 0..25 {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }

    workspace.read_with(cx, |ws, _cx| {
        assert!(
            ws.error_toasts().is_empty(),
            "Error toast should have auto-dismissed past 30 s",
        );
    });
}

/// Pane spawn is core enough to warrant *both* surfaces (status bar
/// pin + transient toast). Regression-guards the wiring so a future
/// refactor can't quietly drop one of them.
#[gpui::test]
async fn report_pane_error_fills_status_bar_and_toast(cx: &mut TestAppContext) {
    use crate::workspace::pane::PaneSpawnError;
    use daruda_terminal::pty::PtyError;

    let (_window, workspace) = build_workspace(cx);

    workspace.update(cx, |ws, cx| {
        ws.report_pane_error(
            "Add tab",
            PaneSpawnError::Pty(PtyError::SpawnShell("synthetic shell failure".into())),
            cx,
        );
    });

    workspace.read_with(cx, |ws, _cx| {
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

        let toasts: Vec<_> = ws.error_toasts().iter().collect();
        assert_eq!(toasts.len(), 1, "toast should fire alongside the pin");
        assert_eq!(toasts[0].report.title, "Pane spawn failed: Add tab");
        assert_eq!(toasts[0].report.severity, ErrorSeverity::Error);
        assert_eq!(toasts[0].report.dedup_key.as_deref(), Some("pane.spawn"));

        // History records it for the (future) "Show recent errors"
        // command palette entry.
        assert_eq!(ws.error_history().len(), 1);
    });
}
