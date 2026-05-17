//! TaskEdit pane — Tab cycle wiring regression test.
//!
//! Each input / select slot in the TaskEdit body picks a Tab cycle
//! position through the third argument of the `crate::ui::*` factory:
//! pass an `isize` to slot the widget into the cycle, or `()` to keep
//! it mouse-only. The `()` form mutates the underlying `FocusHandle`
//! registry to `tab_stop = false`; the `isize` form passed to `select`
//! mutates it the other way (`tab_index = N`). Both mutations land the
//! moment the factory runs at render time, so this test:
//!
//! 1. Opens a fresh draft TaskEdit pane.
//! 2. Calls the body's `render` fn directly — that drives every
//!    `crate::ui::select(...)` / `crate::ui::input(...)` factory in
//!    the body, which is what mutates the focus handle registry.
//! 3. Reads the resulting `tab_index` / `tab_stop` off the state
//!    entities' focus handles.
//!
//! A `select(..., ())` slip on `base_select` would flip `tab_stop` to
//! `false` and leave `tab_index` at the default `0`; both assertions
//! below would catch it.
//!
//! Coverage limit: `crate::ui::input(state, cx, isize)` and
//! `markdown_editor(state).tab_index(isize)` set the Tab position on
//! the rendered `Input` element (not the `FocusHandle` registry), so
//! their slot order is only observable post-paint via the dispatch
//! tree — out of reach of a unit test. The tab-stop-vs-skip choice IS
//! registry-visible; that's what this test pins.

use gpui::{AppContext as _, Focusable as _, TestAppContext};

use super::build_workspace;

#[gpui::test]
async fn task_edit_pane_tab_cycle_wires_base_select(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.open_task_edit_pane(None, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    // Drive the body's render fn so every `crate::ui::*` factory
    // call gets a chance to mutate the focus handle registry.
    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws.panes.last().expect("open_task_edit_pane pushed a pane");
            let pane_id = pane.id;
            let te = match &pane.content {
                crate::workspace::pane::PaneContent::TaskEditPane(te) => te,
                _ => panic!("expected TaskEdit pane"),
            };
            let _ = crate::workspace::right_panel::task_edit_pane::render(pane_id, te, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, cx| {
        let pane = ws.panes.last().unwrap();
        let te = match &pane.content {
            crate::workspace::pane::PaneContent::TaskEditPane(te) => te,
            _ => panic!("expected TaskEdit pane"),
        };

        let base_handle = te.base_select.read(cx).focus_handle(cx);
        assert!(
            base_handle.tab_stop,
            "base_select must participate in the Tab cycle — \
             a `select(..., ())` slip flips tab_stop to false"
        );
        assert_eq!(
            base_handle.tab_index, 3,
            "base_select must occupy Tab slot 3 (title=0, branch=1, \
             prompt=2, base=3, notes=4, new_subtask=5)"
        );

        // The other text-input slots all default to tab_stop=true
        // from `InputState::new`. A slip to `crate::ui::input(state,
        // cx, ())` on any of them would flip tab_stop to false; the
        // assertions below guard against that.
        for (name, handle) in [
            ("title_input", te.title_input.read(cx).focus_handle(cx)),
            ("branch_input", te.branch_input.read(cx).focus_handle(cx)),
            ("prompt_state", te.prompt_state.read(cx).focus_handle(cx)),
            ("notes_state", te.notes_state.read(cx).focus_handle(cx)),
            (
                "new_subtask_input",
                te.new_subtask_input.read(cx).focus_handle(cx),
            ),
        ] {
            assert!(handle.tab_stop, "{name} must participate in the Tab cycle");
        }
    });
}
