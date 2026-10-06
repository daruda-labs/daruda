//! Page navigation must not replace the utility dock or the active worktree.

use super::build_workspace;
use crate::workspace::pages::Page;
use daruda_store::project::{LeftDockView, RightDockView, WorkspacePage};
use gpui::{AppContext as _, Modifiers, TestAppContext, VisualTestContext};

#[gpui::test]
async fn task_status_controls_preserve_scope_and_clear_without_switching_worktree(
    cx: &mut TestAppContext,
) {
    use daruda_store::tasks::{TaskFilter, TaskScope};
    let (window, workspace) = build_workspace(cx);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            use gpui::BorrowAppContext as _;
            let mut task = daruda_store::tasks::Task::new(
                Default::default(),
                "Visible task".into(),
                String::new(),
                None,
            );
            task.state = daruda_store::tasks::TaskState::Running {
                worktree_path: "/tmp".into(),
            };
            cx.update_global::<crate::agent::tasks_global::GlobalTasks, _>(|tasks, _| {
                tasks.add(task);
            });
            ws.set_task_scope(TaskScope::AllProjects, cx);
            ws.open_page(Page::Tasks, window, cx);
        });
        window.refresh();
    });
    vcx.run_until_parked();
    let active = workspace.read_with(&vcx, |ws, _| ws.active);
    let running = vcx
        .debug_bounds("task-status-2")
        .expect("Running tab is visible");
    vcx.simulate_click(running.center(), Modifiers::default());
    vcx.run_until_parked();
    workspace.read_with(&vcx, |ws, _| {
        assert_eq!(ws.task_filter, TaskFilter::Running);
        assert_eq!(ws.task_scope, TaskScope::AllProjects);
        assert_eq!(ws.active, active);
    });
    let clear = vcx
        .debug_bounds("task-clear-filters")
        .expect("a nonempty filtered list has a recovery action");
    vcx.simulate_click(clear.center(), Modifiers::default());
    vcx.run_until_parked();
    workspace.read_with(&vcx, |ws, _| {
        assert_eq!(ws.task_filter, TaskFilter::All);
        assert_eq!(ws.task_scope, TaskScope::AllProjects);
        assert_eq!(ws.active, active);
    });
}

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
async fn task_groups_collapse_and_reopen_without_changing_filters(cx: &mut TestAppContext) {
    use crate::agent::tasks_global::GlobalTasks;
    use crate::workspace::right_dock::tasks::{TaskGroupKey, TaskGrouping};
    use daruda_store::tasks::{Task, TaskFilter, TaskScope};
    use gpui::BorrowAppContext as _;
    let (window, workspace) = build_workspace(cx);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    let mut task = Task::new(
        Default::default(),
        "Fold this task".into(),
        String::new(),
        None,
    );
    task.id = "group-fixture".into();
    vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            cx.update_global::<GlobalTasks, _>(|tasks, _| {
                tasks.add(task);
            });
            ws.set_task_scope(TaskScope::AllProjects, cx);
            ws.set_task_grouping(TaskGrouping::Status, cx);
            ws.open_page(Page::Tasks, window, cx);
        });
        window.refresh();
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("task-row-group-fixture").is_some());
    for width in [800.0, 1280.0] {
        vcx.update(|window, _| {
            window.resize(gpui::size(gpui::px(width), gpui::px(800.0)));
            window.refresh();
        });
        vcx.run_until_parked();
        let heading = vcx.debug_bounds("task-project-column").unwrap();
        let project = vcx.debug_bounds("task-project-group-fixture").unwrap();
        let title = vcx.debug_bounds("task-title-group-fixture").unwrap();
        assert_eq!(
            heading.left(),
            project.left(),
            "project columns align at {width}px"
        );
        assert!(
            title.right() <= project.left(),
            "title cannot overlap the project"
        );
        assert!(title.size.width >= gpui::px(crate::ui::theme::TASK_TABLE_TITLE_MIN_W));
    }
    for expected_open in [false, true] {
        let header = vcx
            .debug_bounds("task-group-0")
            .expect("Backlog group header");
        vcx.simulate_click(header.center(), Modifiers::default());
        vcx.run_until_parked();
        workspace.read_with(&vcx, |ws, _| {
            assert_eq!(
                ws.task_groups
                    .is_open(TaskGroupKey::Status(TaskFilter::Backlog)),
                expected_open
            );
            assert_eq!(ws.task_filter, TaskFilter::All);
            assert_eq!(ws.task_scope, TaskScope::AllProjects);
        });
        assert_eq!(
            vcx.debug_bounds("task-row-group-fixture").is_some(),
            expected_open
        );
    }
    let row = vcx.debug_bounds("task-row-group-fixture").unwrap();
    vcx.simulate_click(row.center(), Modifiers::default());
    vcx.run_until_parked();
    workspace.read_with(&vcx, |ws, _| {
        assert_eq!(
            ws.active_page(),
            None,
            "the row opens the existing editor, not a side panel"
        );
        let pane = ws.active_runtime().focused_pane_id;
        assert_eq!(
            ws.active_runtime()
                .panes
                .iter()
                .find(|p| p.id == pane)
                .unwrap()
                .task_edit_content()
                .unwrap()
                .task_id
                .as_deref(),
            Some("group-fixture")
        );
    });
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
