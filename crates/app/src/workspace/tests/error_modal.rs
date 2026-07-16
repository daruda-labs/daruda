//! Tests for the Details modal — `ErrorReportModal`.
//!
//! Guards three properties: the modal captures the report verbatim at
//! construction; `Copy report` writes the full plain-text rendering
//! (including the system-info trailer) to the clipboard; the `Copied`
//! confirmation reverts after the 1 s window.

use std::time::Duration;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{AppContext as _, ClipboardItem, TestAppContext};

use super::build_workspace;
use crate::workspace::error::modal::ErrorReportModal;

fn synthetic_report() -> ErrorReport {
    ErrorReport::new("PTY died")
        .severity(ErrorSeverity::Error)
        .message("broken pipe")
        .at("crates/app/src/pty.rs", 229)
        .with_context("session", "tab-3-pane-1")
        .dedup("pty.writer")
        .build()
}

#[gpui::test]
async fn modal_captures_full_report_at_construction(cx: &mut TestAppContext) {
    let (window_handle, _workspace) = build_workspace(cx);
    let report = synthetic_report();
    let original_title = report.title.clone();

    let modal = cx
        .update_window(window_handle.into(), |_, window, cx| {
            cx.new(|cx_modal| ErrorReportModal::new(report.clone(), window, cx_modal))
        })
        .unwrap();

    modal.read_with(cx, |m, _cx| {
        assert_eq!(m.report().title, original_title);
        let body = m.body_text().as_ref();
        assert!(
            body.contains("[daruda] PTY died"),
            "body header should match plain-text rendering",
        );
        assert!(
            body.contains("broken pipe"),
            "body should embed the message",
        );
        assert!(
            body.contains("session"),
            "body should embed the context table",
        );
        // System-info trailer — version + os + arch only.
        assert!(
            body.contains(std::env::consts::OS),
            "body trailer should carry OS",
        );
    });
}

#[gpui::test]
async fn copy_report_writes_full_plain_text_to_clipboard(cx: &mut TestAppContext) {
    let (window_handle, _workspace) = build_workspace(cx);
    let report = synthetic_report();
    let expected_body = report.to_plain_text();

    let modal = cx
        .update_window(window_handle.into(), |_, window, cx| {
            cx.new(|cx_modal| ErrorReportModal::new(report, window, cx_modal))
        })
        .unwrap();

    // Write something else first so we can verify the modal overwrites.
    cx.update(|cx| {
        cx.write_to_clipboard(ClipboardItem::new_string("sentinel".to_string()));
    });

    cx.update(|cx| {
        modal.update(cx, |m, cx| m.copy_to_clipboard_for_test(cx));
    });

    let actual = cx.read_from_clipboard().expect("clipboard populated");
    let text = actual.text().expect("clipboard item is text");
    assert_eq!(text, expected_body);

    modal.read_with(cx, |m, _cx| {
        assert!(m.copied(), "Copied confirmation should be active");
    });
}

#[gpui::test]
async fn copied_label_reverts_after_one_second(cx: &mut TestAppContext) {
    let (window_handle, _workspace) = build_workspace(cx);
    let report = synthetic_report();

    let modal = cx
        .update_window(window_handle.into(), |_, window, cx| {
            cx.new(|cx_modal| ErrorReportModal::new(report, window, cx_modal))
        })
        .unwrap();

    cx.update(|cx| {
        modal.update(cx, |m, cx| m.copy_to_clipboard_for_test(cx));
    });

    modal.read_with(cx, |m, _cx| {
        assert!(m.copied(), "Copied label should be up immediately");
    });

    // Past the 1 s revert window — virtual clock so the test is
    // deterministic.
    cx.executor().advance_clock(Duration::from_millis(1500));
    cx.run_until_parked();

    modal.read_with(cx, |m, _cx| {
        assert!(
            !m.copied(),
            "Copied label should have reverted to Copy report",
        );
    });
}
