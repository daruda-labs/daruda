use super::*;

use crate::workspace::main_area::file_view_pane::{CharPos, CharSelection};

// ================================================================
// File-viewer drag-select regression tests
//
// Phase 3 MVU refactor: state transitions for mouse-down and
// mouse-drag are now owned by `Workspace::file_view_mouse_down` and
// `Workspace::file_view_mouse_drag`. These tests verify that each
// branch in both methods behaves identically to the pre-refactor
// View closure.
// ================================================================

/// Build a Workspace rooted at a fresh tempdir with one real file.
/// Opens the file viewer for "a.txt" so `focused_file_view()` is
/// non-None by the time the test body runs.
fn build_workspace_with_open_file(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<Workspace>,
    gpui::Entity<Workspace>,
    tempfile::TempDir,
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::write(root.join("a.txt"), b"hello world").unwrap();

    crate::test_support::init_gpui_component(cx);
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(&root);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();

    // Load the file tree and open the file viewer.
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_files_entry(id, std::path::PathBuf::from("a.txt"), window, cx);
        });
    })
    .unwrap();

    (wh, ws, temp)
}

// ----------------------------------------------------------------
// Test 1 — plain click sets anchor, selection, and drag flag
// ----------------------------------------------------------------

#[gpui::test]
async fn mouse_down_clears_anchor_and_starts_drag(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_open_file(cx);

    ws.update(cx, |ws, cx| {
        ws.file_view_mouse_down(CharPos { row: 0, byte: 5 }, false, cx);
    });

    ws.read_with(cx, |ws, _| {
        let fv = ws.focused_file_view().expect("viewer open");
        assert_eq!(fv.char_anchor, Some(CharPos { row: 0, byte: 5 }));
        assert_eq!(
            fv.char_selection,
            Some(CharSelection {
                anchor: CharPos { row: 0, byte: 5 },
                active: CharPos { row: 0, byte: 5 },
            })
        );
        assert!(fv.is_drag_selecting);
    });
}

// ----------------------------------------------------------------
// Test 2 — shift-click extends selection without starting drag
// ----------------------------------------------------------------

#[gpui::test]
async fn shift_click_extends_selection_without_starting_drag(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_open_file(cx);

    // Prime: plain click at (0, 0).
    ws.update(cx, |ws, cx| {
        ws.file_view_mouse_down(CharPos { row: 0, byte: 0 }, false, cx);
    });
    // Drop the drag flag so we can observe shift-click in isolation.
    ws.update(cx, |ws, _cx| {
        if let Some(fv) = ws.focused_file_view_mut() {
            fv.is_drag_selecting = false;
        }
    });

    // Act: shift-click at (0, 10).
    ws.update(cx, |ws, cx| {
        ws.file_view_mouse_down(CharPos { row: 0, byte: 10 }, true, cx);
    });

    ws.read_with(cx, |ws, _| {
        let fv = ws.focused_file_view().expect("viewer open");
        assert_eq!(
            fv.char_anchor,
            Some(CharPos { row: 0, byte: 0 }),
            "anchor unchanged by shift-click"
        );
        assert_eq!(
            fv.char_selection.as_ref().map(|s| s.active),
            Some(CharPos { row: 0, byte: 10 })
        );
        assert!(!fv.is_drag_selecting, "shift-click does not start a drag");
    });
}

// ----------------------------------------------------------------
// Test 3 — releasing the button resets is_drag_selecting
// ----------------------------------------------------------------

#[gpui::test]
async fn drag_released_resets_is_drag_selecting(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_open_file(cx);

    // Prime: start a drag.
    ws.update(cx, |ws, cx| {
        ws.file_view_mouse_down(CharPos { row: 0, byte: 3 }, false, cx);
    });

    // Act: mouse-move with button released (still_pressed = false).
    ws.update(cx, |ws, cx| {
        ws.file_view_mouse_drag(
            CharPos { row: 0, byte: 7 },
            false, // still_pressed
            true,  // hovered
            cx,
        );
    });

    ws.read_with(cx, |ws, _| {
        let fv = ws.focused_file_view().expect("viewer open");
        assert!(!fv.is_drag_selecting);
        // The selection established by the initial mouse-down is preserved.
        assert!(fv.char_selection.is_some());
    });
}

// ----------------------------------------------------------------
// Test 4 — drag outside the hitbox does not update selection
// ----------------------------------------------------------------

#[gpui::test]
async fn drag_outside_hitbox_does_not_update_selection(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_open_file(cx);

    // Prime: start a drag.
    ws.update(cx, |ws, cx| {
        ws.file_view_mouse_down(CharPos { row: 0, byte: 3 }, false, cx);
    });

    let baseline = ws.read_with(cx, |ws, _| {
        ws.focused_file_view().unwrap().char_selection.clone()
    });

    // Act: drag while not hovered (hovered = false).
    ws.update(cx, |ws, cx| {
        ws.file_view_mouse_drag(
            CharPos { row: 0, byte: 50 },
            true,  // still_pressed
            false, // hovered = false
            cx,
        );
    });

    ws.read_with(cx, |ws, _| {
        let fv = ws.focused_file_view().expect("viewer open");
        assert_eq!(fv.char_selection, baseline, "selection unchanged");
        assert!(fv.is_drag_selecting, "drag flag still set while button held");
    });
}
