use super::*;

// ---- Worktrees (W-2) ----

#[gpui::test]
fn test_workspace_without_project_has_no_worktrees(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, _| {
        assert!(ws.worktrees.is_empty());
    });
}

#[gpui::test]
fn test_workspace_with_project_bootstraps_one_default_worktree(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_bootstrap_wt");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.worktrees.len(), 1);
        assert_eq!(ws.worktrees[0].id, 0);
        assert_eq!(ws.active_worktree_id, 0);
        assert_eq!(
            ws.worktrees[0].kind,
            daruda_store::project::WorktreeKind::Default
        );
        assert_eq!(
            ws.worktrees[0].path,
            std::path::PathBuf::from("/tmp/test_bootstrap_wt")
        );
    });
}

#[gpui::test]
fn test_activate_worktree_requires_existing_id(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_wt_activate");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            // Nonexistent id → no-op.
            ws.activate_worktree(99, window, cx);
            assert_eq!(ws.active_worktree_id, 0);
            // Self id → no-op (already active).
            ws.activate_worktree(0, window, cx);
            assert_eq!(ws.active_worktree_id, 0);
        });
    })
    .unwrap();
}

#[gpui::test]
fn test_worktree_removable_excludes_main_and_default(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |_ws, _| {
        let default_wt =
            crate::worktree::Worktree::default_for_project(0, std::path::PathBuf::from("/tmp"));
        assert!(!Workspace::worktree_removable(&default_wt));

        let main_git = crate::worktree::Worktree::git(
            0,
            std::path::PathBuf::from("/tmp/repo"),
            Some("main".into()),
            std::path::PathBuf::from("/tmp/repo"),
            std::path::PathBuf::from("/tmp/repo"),
            0,
        );
        assert!(!Workspace::worktree_removable(&main_git));

        let linked = crate::worktree::Worktree::git(
            1,
            std::path::PathBuf::from("/tmp/repo-feat"),
            Some("feat".into()),
            std::path::PathBuf::from("/tmp/repo"),
            std::path::PathBuf::from("/tmp/repo-feat"),
            1,
        );
        assert!(Workspace::worktree_removable(&linked));
    });
}

#[gpui::test]
fn test_git_repo_root_returns_none_for_non_git_workspace(cx: &mut TestAppContext) {
    // Non-git Default worktree → git_repo_root() is the gate the
    // `[+]` button uses to short-circuit before opening the modal.
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_modal_non_git");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert!(ws.git_repo_root().is_none());
    });
}

#[gpui::test]
fn test_validate_remove_worktree_rejects_unknown_id(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_remove_unknown");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        let err = ws.validate_remove_worktree(999).unwrap_err();
        assert!(err.contains("not found"));
    });
}

#[gpui::test]
fn test_validate_remove_worktree_rejects_default_kind(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_remove_default");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        // Bootstrapped default worktree at id 0 — not removable.
        let err = ws.validate_remove_worktree(0).unwrap_err();
        assert!(err.contains("cannot be removed"));
    });
}

#[gpui::test]
fn test_validate_remove_worktree_rejects_default(cx: &mut TestAppContext) {
    // Default-kind worktree → not removable. The `×` click handler in
    // sidebar/worktrees/list.rs uses validate_remove_worktree() to
    // decide whether to even open the modal.
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_remove_main_noop");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        let err = ws.validate_remove_worktree(0).unwrap_err();
        assert!(err.contains("cannot be removed"));
    });
}

#[gpui::test]
fn test_activate_worktree_swaps_tabs(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_wt_swap");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(
            &config,
            Some(project.clone()),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();

    // Seed a second worktree with empty runtime so we can swap into it.
    ws.update(cx, |ws, _cx| {
        ws.worktrees
            .push(crate::worktree::Worktree::default_for_project(
                1,
                std::path::PathBuf::from("/tmp/test_wt_swap_b"),
            ));
    });

    // Swap to worktree 1 → the previous runtime lands in the
    // inactive map, and activate_worktree lazy-spawns a new pane
    // rooted at the target worktree's path (so the viewport isn't
    // empty).
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(ws.tabs.len(), 1);
            assert_eq!(ws.panes.len(), 1);
            ws.activate_worktree(1, window, cx);
            assert_eq!(ws.active_worktree_id, 1);
            // Lazy seed: exactly one tab/pane materialized.
            assert_eq!(ws.tabs.len(), 1);
            assert_eq!(ws.panes.len(), 1);
            assert_eq!(ws.inactive_worktree_runtimes.len(), 1);
            let stashed = ws.inactive_worktree_runtimes.get(&0).unwrap();
            assert_eq!(stashed.tabs.len(), 1);
            assert_eq!(stashed.panes.len(), 1);
        });
    })
    .unwrap();

    // Swap back → the original runtime rehydrates (same PTY tasks,
    // same pane ids). Worktree 1's freshly-spawned runtime parks in
    // the inactive map.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.activate_worktree(0, window, cx);
            assert_eq!(ws.active_worktree_id, 0);
            assert_eq!(ws.tabs.len(), 1);
            assert_eq!(ws.panes.len(), 1);
            let stashed = ws.inactive_worktree_runtimes.get(&1).unwrap();
            // Worktree 1 carries its lazy-spawned pane now.
            assert_eq!(stashed.tabs.len(), 1);
            assert_eq!(stashed.panes.len(), 1);
        });
    })
    .unwrap();
}

#[gpui::test]
fn test_save_state_serializes_inactive_worktree_runtime(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_wt_save_inactive");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(
            &config,
            Some(project.clone()),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();

    // Add a second worktree, push into it, swap back — now inactive
    // worktree 1 has meta only (no tabs). Verify save captures both
    // without duplication.
    ws.update(cx, |ws, _cx| {
        ws.worktrees
            .push(crate::worktree::Worktree::default_for_project(
                1,
                std::path::PathBuf::from("/tmp/test_wt_save_inactive_b"),
            ));
    });

    let state = ws
        .read_with(cx, |ws, app_cx| ws.save_state(app_cx))
        .unwrap();
    assert_eq!(state.worktrees.len(), 2);
    // Active worktree 0 has its tab.
    let wt0 = state.worktrees.iter().find(|w| w.id == 0).unwrap();
    assert!(!wt0.tabs.is_empty());
    // Inactive worktree 1 has no runtime → serialized tabs empty.
    let wt1 = state.worktrees.iter().find(|w| w.id == 1).unwrap();
    assert!(wt1.tabs.is_empty());
    assert!(state.tabs.is_empty());
}

#[gpui::test]
fn test_restore_state_rebuilds_all_worktrees(cx: &mut TestAppContext) {
    // Create the worktree directories so the path-exists check in
    // restore_state doesn't skip the inactive worktree layout rebuild.
    std::fs::create_dir_all("/tmp/test_wt_restore_all/main").unwrap();
    std::fs::create_dir_all("/tmp/test_wt_restore_all/side").unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_wt_restore_all");
    let wh = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project(
            &config,
            Some(project.clone()),
            fresh_test_data_dir(),
            window,
            cx,
        );
        let make_tab = |pane_id: u64, cwd: &str| daruda_store::project::SerializedTab {
            layout: daruda_store::project::SerializedLayout::Leaf {
                pane_id,
                cwd: Some(std::path::PathBuf::from(cwd)),
                file: None,
            },
            last_focused_pane: pane_id,
            user_label: None,
        };
        let state = daruda_store::project::ProjectState {
            root: std::path::PathBuf::from("/tmp/test_wt_restore_all"),
            worktrees: vec![
                daruda_store::project::SerializedWorktree {
                    id: 0,
                    kind: daruda_store::project::WorktreeKind::Default,
                    path: std::path::PathBuf::from("/tmp/test_wt_restore_all/main"),
                    name: None,
                    tab_order: 0,
                    is_unread: false,
                    last_activity: 0,
                    tabs: vec![make_tab(100, "/tmp/test_wt_restore_all/main")],
                    active_tab_index: 0,
                    base_ref: None,
                    description: None,
                },
                daruda_store::project::SerializedWorktree {
                    id: 5,
                    kind: daruda_store::project::WorktreeKind::Default,
                    path: std::path::PathBuf::from("/tmp/test_wt_restore_all/side"),
                    name: Some("Side".into()),
                    tab_order: 1,
                    is_unread: true,
                    last_activity: 0,
                    tabs: vec![
                        make_tab(200, "/tmp/test_wt_restore_all/side"),
                        make_tab(201, "/tmp/test_wt_restore_all/side"),
                    ],
                    active_tab_index: 1,
                    base_ref: None,
                    description: None,
                },
            ],
            active_worktree_id: 5,
            active_sidebar_view: daruda_store::project::LeftSidebarView::default(),
            active_right_sidebar_view: daruda_store::project::RightSidebarView::default(),
            active_usage_window: daruda_store::project::UsageWindow::default(),
            tabs: Vec::new(),
            active_tab_index: 0,
            focused_pane_id: 201,
            docks: daruda_store::project::DockStates::default(),
            window: daruda_store::project::WindowState::default(),
            window_user_label: None,
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
        };
        ws.restore_state(&state, window, cx);
        ws
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        // Active worktree is 5 with 2 tabs → Workspace.tabs mirrors it.
        assert_eq!(ws.active_worktree_id, 5);
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.active_tab_index, 1);
        // Inactive worktree 0 rebuilt into the inactive map with 1 tab.
        assert_eq!(ws.inactive_worktree_runtimes.len(), 1);
        let stashed = ws.inactive_worktree_runtimes.get(&0).unwrap();
        assert_eq!(stashed.tabs.len(), 1);
    });
}

#[gpui::test]
fn test_save_state_captures_bootstrapped_worktree(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_save_wt");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    let state = ws
        .read_with(cx, |ws, app_cx| ws.save_state(app_cx))
        .unwrap();
    assert_eq!(state.worktrees.len(), 1);
    assert_eq!(state.worktrees[0].id, 0);
    assert_eq!(
        state.worktrees[0].path,
        std::path::PathBuf::from("/tmp/test_save_wt")
    );
    assert_eq!(state.active_worktree_id, 0);
    // Tabs live inside the active worktree from W-2 onward.
    assert!(!state.worktrees[0].tabs.is_empty());
    assert!(state.tabs.is_empty());
}

#[gpui::test]
fn test_restore_state_reads_tabs_from_active_worktree(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_wt_tabs");
    let wh = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx);
        let worktree = daruda_store::project::SerializedWorktree {
            id: 0,
            kind: daruda_store::project::WorktreeKind::Default,
            path: std::path::PathBuf::from("/tmp/test_restore_wt_tabs"),
            name: None,
            tab_order: 0,
            is_unread: false,
            last_activity: 0,
            tabs: vec![daruda_store::project::SerializedTab {
                layout: daruda_store::project::SerializedLayout::Leaf {
                    pane_id: 1,
                    cwd: Some(std::path::PathBuf::from("/tmp/test_restore_wt_tabs")),
                    file: None,
                },
                last_focused_pane: 1,
                user_label: None,
            }],
            active_tab_index: 0,
            base_ref: None,
            description: None,
        };
        let state = daruda_store::project::ProjectState {
            root: std::path::PathBuf::from("/tmp/test_restore_wt_tabs"),
            worktrees: vec![worktree],
            active_worktree_id: 0,
            active_sidebar_view: daruda_store::project::LeftSidebarView::default(),
            active_right_sidebar_view: daruda_store::project::RightSidebarView::default(),
            active_usage_window: daruda_store::project::UsageWindow::default(),
            tabs: vec![],
            active_tab_index: 0,
            focused_pane_id: 1,
            docks: daruda_store::project::DockStates::default(),
            window: daruda_store::project::WindowState::default(),
            window_user_label: None,
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
        };
        ws.restore_state(&state, window, cx);
        ws
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.worktrees.len(), 1);
        assert_eq!(ws.active_worktree_id, 0);
    });
}

#[gpui::test]
fn test_restore_state_clamps_stale_active_worktree_id(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_clamp_id");
    let wh = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx);
        let worktree = daruda_store::project::SerializedWorktree::default_for_path(
            3,
            std::path::PathBuf::from("/tmp/test_restore_clamp_id"),
        );
        let state = daruda_store::project::ProjectState {
            root: std::path::PathBuf::from("/tmp/test_restore_clamp_id"),
            worktrees: vec![worktree],
            active_worktree_id: 999, // stale — only id 3 exists
            active_sidebar_view: daruda_store::project::LeftSidebarView::default(),
            active_right_sidebar_view: daruda_store::project::RightSidebarView::default(),
            active_usage_window: daruda_store::project::UsageWindow::default(),
            tabs: vec![],
            active_tab_index: 0,
            focused_pane_id: 0,
            docks: daruda_store::project::DockStates::default(),
            window: daruda_store::project::WindowState::default(),
            window_user_label: None,
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
        };
        ws.restore_state(&state, window, cx);
        ws
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active_worktree_id, 3);
    });
}

#[gpui::test]
fn test_sidebar_view_defaults_to_worktrees(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.left_sidebar_view,
            daruda_store::project::LeftSidebarView::Worktrees
        );
    });
}

#[gpui::test]
fn test_set_sidebar_view_switches_and_notifies(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        ws.set_sidebar_view(daruda_store::project::LeftSidebarView::GitChanges, cx);
        assert_eq!(
            ws.left_sidebar_view,
            daruda_store::project::LeftSidebarView::GitChanges
        );
        ws.set_sidebar_view(daruda_store::project::LeftSidebarView::Files, cx);
        assert_eq!(
            ws.left_sidebar_view,
            daruda_store::project::LeftSidebarView::Files
        );
        // No-op when already on that view (shouldn't panic).
        ws.set_sidebar_view(daruda_store::project::LeftSidebarView::Files, cx);
        assert_eq!(
            ws.left_sidebar_view,
            daruda_store::project::LeftSidebarView::Files
        );
    });
}

#[gpui::test]
fn test_save_state_captures_active_sidebar_view(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_save_sidebar_view");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    ws.update(cx, |ws, cx| {
        ws.set_sidebar_view(daruda_store::project::LeftSidebarView::Files, cx);
    });
    let state = ws
        .read_with(cx, |ws, app_cx| ws.save_state(app_cx))
        .unwrap();
    assert_eq!(
        state.active_sidebar_view,
        daruda_store::project::LeftSidebarView::Files
    );
}

#[gpui::test]
fn test_restore_state_applies_active_sidebar_view(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_sidebar_view");
    let wh = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx);
        let state = daruda_store::project::ProjectState {
            root: std::path::PathBuf::from("/tmp/test_restore_sidebar_view"),
            worktrees: Vec::new(),
            active_worktree_id: 0,
            active_sidebar_view: daruda_store::project::LeftSidebarView::GitChanges,
            active_right_sidebar_view: daruda_store::project::RightSidebarView::default(),
            active_usage_window: daruda_store::project::UsageWindow::default(),
            tabs: vec![],
            active_tab_index: 0,
            focused_pane_id: 0,
            docks: daruda_store::project::DockStates::default(),
            window: daruda_store::project::WindowState::default(),
            window_user_label: None,
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
        };
        ws.restore_state(&state, window, cx);
        ws
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.left_sidebar_view,
            daruda_store::project::LeftSidebarView::GitChanges
        );
    });
}

#[gpui::test]
fn test_save_state_captures_active_right_sidebar_view(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_save_right_sidebar_view");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    ws.update(cx, |ws, cx| {
        ws.set_right_sidebar_view(daruda_store::project::RightSidebarView::Tools, cx);
    });
    let state = ws
        .read_with(cx, |ws, app_cx| ws.save_state(app_cx))
        .unwrap();
    assert_eq!(
        state.active_right_sidebar_view,
        daruda_store::project::RightSidebarView::Tools
    );
}

#[gpui::test]
fn test_restore_state_applies_active_right_sidebar_view(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_right_sidebar_view");
    let wh = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx);
        let state = daruda_store::project::ProjectState {
            root: std::path::PathBuf::from("/tmp/test_restore_right_sidebar_view"),
            worktrees: Vec::new(),
            active_worktree_id: 0,
            active_sidebar_view: daruda_store::project::LeftSidebarView::default(),
            active_right_sidebar_view: daruda_store::project::RightSidebarView::Tasks,
            active_usage_window: daruda_store::project::UsageWindow::default(),
            tabs: vec![],
            active_tab_index: 0,
            focused_pane_id: 0,
            docks: daruda_store::project::DockStates::default(),
            window: daruda_store::project::WindowState::default(),
            window_user_label: None,
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
        };
        ws.restore_state(&state, window, cx);
        ws
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.right_sidebar_view,
            daruda_store::project::RightSidebarView::Tasks
        );
    });
}

#[gpui::test]
fn test_save_state_captures_active_usage_window(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_save_usage_window");
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));
    let _ = wh.update(cx, |_root, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.set_usage_window(daruda_store::project::UsageWindow::Last5h, window, cx);
        });
    });
    let state = ws
        .read_with(cx, |ws, app_cx| ws.save_state(app_cx))
        .unwrap();
    assert_eq!(
        state.active_usage_window,
        daruda_store::project::UsageWindow::Last5h
    );
    // `set_usage_window` must also keep the dropdown in sync — any
    // future caller that bypasses the Confirm event path (action
    // handler, keyboard shortcut, programmatic restore, …) needs the
    // visible UI to follow `Workspace::usage_window`.
    ws.read_with(cx, |ws, cx_inner| {
        let v = ws.claude.usage_select.read(cx_inner).selected_value();
        assert_eq!(
            v.map(|s| s.as_ref()),
            Some(daruda_store::project::UsageWindow::Last5h.slug())
        );
    });
}

#[gpui::test]
fn test_restore_state_applies_active_usage_window(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_usage_window");
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));
    let _ = wh.update(cx, |_root, window, cx| {
        ws.update(cx, |ws, cx| {
            let state = daruda_store::project::ProjectState {
                root: std::path::PathBuf::from("/tmp/test_restore_usage_window"),
                worktrees: Vec::new(),
                active_worktree_id: 0,
                active_sidebar_view: daruda_store::project::LeftSidebarView::default(),
                active_right_sidebar_view: daruda_store::project::RightSidebarView::default(),
                active_usage_window: daruda_store::project::UsageWindow::Last24h,
                tabs: vec![],
                active_tab_index: 0,
                focused_pane_id: 0,
                docks: daruda_store::project::DockStates::default(),
                window: daruda_store::project::WindowState::default(),
                window_user_label: None,
                font_size: 13.0,
                vertical_spacing: 1.0,
                horizontal_spacing: 1.0,
            };
            ws.restore_state(&state, window, cx);
        });
    });
    ws.read_with(cx, |ws, cx_inner| {
        assert_eq!(
            ws.claude.usage_window,
            daruda_store::project::UsageWindow::Last24h
        );
        // Picker selection follows the restored state.
        let v = ws.claude.usage_select.read(cx_inner).selected_value();
        assert_eq!(
            v.map(|s| s.as_ref()),
            Some(daruda_store::project::UsageWindow::Last24h.slug())
        );
    });
}

#[gpui::test]
fn test_workspace_renders_with_docks_open(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        ws.left_dock.update(cx, |d, _| d.toggle());
        ws.bottom_dock.update(cx, |d, _| d.toggle());
        ws.right_dock.update(cx, |d, _| d.toggle());
        assert!(ws.left_dock.read(cx).is_open);
        assert!(ws.bottom_dock.read(cx).is_open);
        assert!(ws.right_dock.read(cx).is_open);
    });
}
