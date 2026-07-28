//! Tests for `Workspace` annotation ops.
//!
//! Happy path: `add_annotation` round-trips through the pane's
//! TerminalSession into `annotations_in_range`. Error path: a missing
//! target pane routes the failure through `report_error` (asserted via
//! the history entry, not a return value).

use super::*;
use daruda_store::observability::error_report::ErrorSeverity;
use daruda_terminal::session::LineBufferPosition;
use daruda_terminal::session::interval_tree::{LineCoord, LineRange};
use gpui::{Point, px};

fn first_terminal_pane_id(ws: &Workspace) -> crate::workspace::main_area::pane_tree::PaneId {
    ws.active_runtime()
        .panes
        .iter()
        .find(|p| {
            matches!(
                p.content,
                crate::workspace::main_area::pane::PaneContent::Terminal(_)
            )
        })
        .map(|p| p.id)
        .expect("workspace boots with at least one terminal pane")
}

fn single_line_range_at_zero() -> LineRange {
    let c = LineCoord::Buffered(LineBufferPosition { abs_index: 0 });
    LineRange::new(c, c)
}

#[gpui::test]
async fn add_annotation_round_trips_into_session(cx: &mut TestAppContext) {
    let (_window, workspace) = build_workspace(cx);

    workspace.update(cx, |ws, cx| {
        let pane_id = first_terminal_pane_id(ws);
        ws.add_annotation(
            pane_id,
            single_line_range_at_zero(),
            "hello".to_string(),
            cx,
        );
    });

    workspace.read_with(cx, |ws, cx| {
        let pane = ws
            .active_runtime()
            .panes
            .iter()
            .find(|p| {
                matches!(
                    p.content,
                    crate::workspace::main_area::pane::PaneContent::Terminal(_)
                )
            })
            .expect("terminal pane");
        let crate::workspace::main_area::pane::PaneContent::Terminal(t) = &pane.content else {
            panic!("expected terminal pane");
        };
        let session = t.view.read(cx).session();
        let hits = session.annotations_in_range(single_line_range_at_zero());
        assert_eq!(hits.len(), 1, "session should expose the new annotation");
        assert_eq!(hits[0].1.text, "hello");
    });
}

#[gpui::test]
async fn add_annotation_with_missing_pane_reports_error(cx: &mut TestAppContext) {
    let (_window, workspace) = build_workspace(cx);

    // PaneId is a monotonic u64; an id far above the next allocation is
    // guaranteed to miss the lookup without mutating workspace state.
    let phantom_pane: crate::workspace::main_area::pane_tree::PaneId = 99_999_u64;

    workspace.update(cx, |ws, cx| {
        ws.add_annotation(
            phantom_pane,
            single_line_range_at_zero(),
            "ghost".to_string(),
            cx,
        );
    });

    workspace.read_with(cx, |ws, _cx| {
        let history = ws.error_history();
        assert!(
            history.iter().any(|r| {
                r.dedup_key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("workspace.annotation.pane_missing"))
                    && r.severity == ErrorSeverity::Info
            }),
            "missing-pane add should surface a report; got {:?}",
            history
                .iter()
                .map(|r| (r.dedup_key.as_deref(), r.severity))
                .collect::<Vec<_>>(),
        );
    });
}

#[gpui::test]
fn open_annotation_context_menu_builds_with_copy_item(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    // Smoke test only: no selection exists on a fresh pane, so this
    // exercises the disabled-Copy / disabled-Add / no-Delete branches of
    // `open_annotation_context_menu` without panicking and confirms the
    // menu still deploys. Asserting the Copy item's *enabled* branch would
    // need a selection set on the pane's `TerminalView`, which has no
    // public test-support setter (selection state is driven by real mouse
    // events) — left as a manual-verification gap, matching the crate's
    // "render paths needing Window rely on manual smoke tests" convention.
    window_handle
        .update(cx, |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane_id = first_terminal_pane_id(ws);
                ws.open_annotation_context_menu(
                    pane_id,
                    Point::new(px(0.), px(0.)),
                    None,
                    window,
                    cx,
                );
                assert!(
                    ws.main_area.popup_menu_deploy.is_some(),
                    "menu with Copy + Add annotation items should deploy"
                );
            });
        })
        .unwrap();
}
