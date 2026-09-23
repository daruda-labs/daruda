//! Page navigation must not replace the utility dock or the active worktree.

use super::build_workspace;
use crate::workspace::pages::Page;
use daruda_store::project::{LeftDockView, RightDockView, WorkspacePage};
use gpui::{AppContext as _, Modifiers, TestAppContext, VisualTestContext};

#[gpui::test]
fn pages_preserve_the_active_lane_and_utility_selection(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let lane = ws.active;
            ws.set_right_dock_view(RightDockView::Skills, cx);
            ws.open_page(Page::Tasks, window, cx);
            assert_eq!(ws.active_page(), Some(Page::Tasks));
            assert_eq!(ws.right_dock_view, RightDockView::Skills);
            ws.set_right_dock_view(RightDockView::Tools, cx);
            assert_eq!(ws.active_page(), Some(Page::Tasks));
            ws.open_page(Page::Flows, window, cx);
            assert_eq!(ws.active_page(), Some(Page::Flows));
            assert_eq!(ws.active, lane);
            assert_eq!(ws.right_dock_view, RightDockView::Tools);
        });
    })
    .unwrap();
}

#[gpui::test]
fn selecting_projects_or_the_same_lane_returns_to_its_content(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.open_page(Page::Tasks, window, cx);
            ws.set_left_dock_view(LeftDockView::Lanes, cx);
            assert_eq!(ws.active_page(), None);
            ws.open_page(Page::Flows, window, cx);
            ws.activate_lane(ws.active, window, cx);
            assert_eq!(ws.active_page(), None);
        });
    })
    .unwrap();
}

#[gpui::test]
fn task_editor_is_visible_after_leaving_the_task_page(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.open_page(Page::Tasks, window, cx);
            ws.open_task_edit_pane(None, window, cx);
            assert_eq!(ws.active_page(), None);
        });
    })
    .unwrap();
}

#[gpui::test]
fn page_selection_survives_restore_with_an_existing_pane(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.set_right_dock_view(RightDockView::Skills, cx);
            ws.open_page(Page::Tasks, window, cx);
            let (state, projects) = ws.snapshot_for_disk(cx);
            assert_eq!(state.active_page, Some(WorkspacePage::Tasks));
            assert_eq!(state.active_right_panel_view, RightDockView::Skills);
            ws.restore_from_disk(&state, &projects, window, cx);
            assert_eq!(ws.active_page(), Some(Page::Tasks));
            // The page no longer borrows the tab's slot, so the tab survives.
            assert_eq!(ws.right_dock_view, RightDockView::Skills);
        });
    })
    .unwrap();
}

fn assert_page_close_preserves_workspace(
    cx: &mut TestAppContext,
    mut close: impl FnMut(&mut VisualTestContext),
) {
    for tab_count in [1, 0, 2] {
        let (window_handle, workspace) = build_workspace(cx);
        let mut vcx = VisualTestContext::from_window(window_handle.into(), cx);
        vcx.cx.update(crate::bind_keys::register_static_bindings);
        vcx.update(|window, cx| {
            workspace.update(cx, |ws, cx| match tab_count {
                0 => ws.empty_active_lane_runtime(window, cx),
                2 => ws.add_tab(window, cx),
                _ => {}
            });
        });
        let before = workspace.read_with(&vcx, |ws, _| {
            let runtime = ws.active_runtime();
            (
                ws.active,
                runtime.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(),
                runtime.panes.iter().map(|pane| pane.id).collect::<Vec<_>>(),
                runtime.focused_pane_id,
            )
        });

        for page in [Page::Tasks, Page::Flows] {
            vcx.update(|window, cx| {
                workspace.update(cx, |ws, cx| ws.open_page(page, window, cx));
                window.refresh();
            });
            vcx.run_until_parked();
            close(&mut vcx);
            vcx.run_until_parked();

            assert!(
                vcx.cx
                    .update(|cx| cx.windows().contains(&window_handle.into())),
                "closing {page:?} with {tab_count} underlying tabs must keep the window open",
            );
            vcx.update(|window, cx| {
                let ws = workspace.read(cx);
                assert_eq!(ws.active_page(), None, "the page itself must close");
                let runtime = ws.active_runtime();
                assert_eq!(
                    (
                        ws.active,
                        runtime.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(),
                        runtime.panes.iter().map(|pane| pane.id).collect::<Vec<_>>(),
                        runtime.focused_pane_id,
                    ),
                    before,
                    "closing a page must not remove hidden worktree content",
                );
                let focus = runtime
                    .panes
                    .iter()
                    .find(|pane| pane.id == runtime.focused_pane_id)
                    .map(|pane| pane.focus_handle(cx))
                    .unwrap_or_else(|| ws.focus_handle.clone());
                assert!(
                    focus.is_focused(window),
                    "focus must return to the worktree"
                );
            });
        }
    }
}

#[gpui::test]
async fn close_shortcut_dismisses_the_page_not_the_hidden_pane(cx: &mut TestAppContext) {
    assert_page_close_preserves_workspace(cx, |vcx| {
        vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-w"
        } else {
            "ctrl-w"
        });
    });
}

#[gpui::test]
async fn close_tab_action_dismisses_the_page_not_the_hidden_tab(cx: &mut TestAppContext) {
    assert_page_close_preserves_workspace(cx, |vcx| {
        vcx.dispatch_action(crate::workspace::CloseTab);
    });
}

#[gpui::test]
async fn page_close_button_preserves_the_window_and_worktree(cx: &mut TestAppContext) {
    assert_page_close_preserves_workspace(cx, |vcx| {
        let bounds = vcx
            .debug_bounds("close-workspace-page")
            .expect("the page close button must be visible");
        vcx.simulate_click(bounds.center(), Modifiers::default());
    });
}
