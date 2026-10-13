use super::*;

// ---------------- Integration ----------------

#[gpui::test]
async fn file_tree_loads_root_and_lazy_children(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| {
        ws.ensure_file_tree(id, cx);
    });
    // Snapshot before the root scan returns.
    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        let tree = ws.lane_file_tree(id).expect("tree");
        let names: Vec<&str> = tree
            .child_entries(tree.root_id)
            .map(|e| e.name.as_str())
            .collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
        assert!(names.contains(&"sub"));
    });
    ws.update(cx, |ws, _cx| {
        let id = ws.active_ref();
        let v2: Arc<_> = ws.cached_or_rebuild_visible(id);
        assert!(
            !Arc::ptr_eq(&v1, &v2),
            "apply_dir_load_result must invalidate the visible cache"
        );
        let names: Vec<&str> = v2.iter().map(|v| v.name.as_str()).collect();
        // Sort puts dirs first ("sub"), then alpha files.
        assert_eq!(names, vec!["sub", "a.txt", "b.txt"]);
    });

    // `sub` has no children loaded yet (UnloadedDir).
    let sub_id = child_id_by_name(&ws, cx, "sub");
    ws.read_with(cx, |ws, _| {
        let tree = ws.lane_file_tree(id).unwrap();
        let entry = tree.entry(sub_id).unwrap();
        assert_eq!(entry.kind, EntryKind::UnloadedDir);
        assert!(tree.child_ids(sub_id).is_empty());
    });

    // Toggle expand → kicks load.
    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    ws.update(cx, |ws, cx| ws.toggle_files_expand(id, sub_id, cx));
    let v2: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    assert!(
        !Arc::ptr_eq(&v1, &v2),
        "toggle_files_expand must invalidate the visible cache"
    );
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        let tree = ws.lane_file_tree(id).unwrap();
        let entry = tree.entry(sub_id).unwrap();
        assert_eq!(entry.kind, EntryKind::Dir);
        let names: Vec<&str> = tree
            .child_entries(sub_id)
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(names, vec!["nested.txt"]);
    });
}

#[gpui::test]
async fn clicking_file_opens_raw_viewer_dedupes_and_selection_moves_independently(
    cx: &mut TestAppContext,
) {
    let (wh, ws, temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();
    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    let a_id = v1.iter().find(|v| v.name == "a.txt").unwrap().entry_id;
    let initial_tab_count = ws.read_with(cx, |ws, _| ws.active_runtime().tabs.len());

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_files_entry(
                id,
                std::path::PathBuf::from("a.txt"),
                crate::workspace::main_area::tab_ops::OpenIntent::Enter,
                window,
                cx,
            );
        });
    })
    .unwrap();
    let v2: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    assert!(
        !Arc::ptr_eq(&v1, &v2),
        "selection change must invalidate the visible cache"
    );

    ws.read_with(cx, |ws, _| {
        let fv = ws.focused_file_view().expect("file viewer open");
        assert_eq!(ws.active, id, "the pane opened in its own lane");
        assert_eq!(fv.path, std::path::PathBuf::from("a.txt"));
        assert_eq!(
            fv.source,
            DiffSource::WorkingTree,
            "Files view always shows the working tree"
        );
        assert!(matches!(
            fv.view_mode,
            crate::workspace::main_area::file_view_pane::FileViewMode::Raw
        ));
    });

    let after_first = ws.read_with(cx, |ws, _| ws.active_runtime().tabs.len());
    assert_eq!(
        after_first,
        initial_tab_count + 1,
        "first click adds a file-viewer tab"
    );

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_files_entry(
                id,
                std::path::PathBuf::from("a.txt"),
                crate::workspace::main_area::tab_ops::OpenIntent::Enter,
                window,
                cx,
            );
        });
    })
    .unwrap();
    let after_second = ws.read_with(cx, |ws, _| ws.active_runtime().tabs.len());
    assert_eq!(
        after_second, after_first,
        "re-clicking the same file does not open a second tab"
    );

    // Focused pane is the file viewer for that path.
    ws.read_with(cx, |ws, _| {
        let fv = ws.focused_file_view().expect("file viewer remains focused");
        assert_eq!(fv.path, std::path::PathBuf::from("a.txt"));
    });

    // Reproduces: mouse-click highlight stayed on the old file after an arrow
    // key. The dock background follows `files_selection`, independently of the
    // already-open file viewer.
    ws.update(cx, |ws, _cx| {
        ws.file_tree.files_selection = Some(a_id);
        ws.invalidate_visible_files_cache(id);
    });

    // Arrow-down: cursor moves off a.txt.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.move_files_selection(1, window, cx));
    })
    .unwrap();

    ws.update(cx, |ws, _cx| {
        let visible = ws.cached_or_rebuild_visible(id);
        let a = visible.iter().find(|v| v.name == "a.txt").unwrap();
        assert!(
            !a.is_keyboard_focused,
            "a.txt must lose keyboard focus after arrow-down"
        );
        // Some other row now holds the cursor.
        assert!(visible.iter().any(|v| v.is_keyboard_focused));
        // The file viewer stays open on a.txt — its tab is
        // independent of the left-dock selection bg.
        assert_eq!(
            ws.focused_file_view().unwrap().path.as_path(),
            std::path::Path::new("a.txt")
        );
    });

    // Capture the current state — the file pane is part of the active
    // lane's tab list.
    let (saved_workspace, saved_projects) =
        ws.read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx));
    assert_eq!(saved_projects.len(), 1);
    assert_eq!(saved_projects[0].lanes.len(), 1);
    let saved_tabs = &saved_projects[0].lanes[0].tabs;
    assert!(
        saved_tabs.iter().any(|t| matches!(
            &t.layout,
            daruda_store::project::SerializedLayout::Leaf {
                content: daruda_store::project::SerializedPaneContent::File(fc),
                ..
            } if fc.path == std::path::Path::new("a.txt")
        )),
        "saved state must include a file leaf for a.txt",
    );

    // Build a fresh workspace at the same project root and apply
    // the saved state. The restored workspace should expose a file
    // pane whose `focused_file_view()` matches the original.
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(temp.path());
    let wh2 = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        );
        ws.restore_from_disk(&saved_workspace, &saved_projects, window, cx);
        ws
    });
    let ws2 = wh2.root(cx).unwrap();
    cx.run_until_parked();

    ws2.read_with(cx, |ws, _| {
        let fv = ws
            .focused_file_view()
            .expect("restored workspace must focus the file pane");
        assert_eq!(fv.path, std::path::PathBuf::from("a.txt"));
        assert_eq!(ws.active, id, "the pane opened in its own lane");
        assert!(matches!(
            fv.view_mode,
            crate::workspace::main_area::file_view_pane::FileViewMode::Raw
        ));
    });
}

// ---------------- Cache invalidation regression ----------------

// ---------------- watcher / dirty flag ----------------

#[gpui::test]
async fn watcher_event_updates_tree_invalidates_cache_and_collapses_git_refresh(
    cx: &mut TestAppContext,
) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    let repo = ws.read_with(cx, |ws, _| ws.active_lanes()[0].path.clone());
    ws.update(cx, |ws, _| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes[0].kind = daruda_store::project::LaneKind::Git {
                branch: Some("main".into()),
                repo_root: repo.clone(),
                worktree_root: repo,
            };
        }
    });
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();
    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));

    // Create a new file on disk; we inject the debounced event
    // directly so the test does not depend on watcher timing.
    let new_path = ws.read_with(cx, |ws, _| ws.active_lanes()[0].path.join("c.txt"));
    std::fs::write(&new_path, b"new").unwrap();
    ws.update(cx, |ws, cx| {
        ws.queue_files_event(
            id,
            crate::files::watcher::DebouncedEvent::Changed {
                paths: vec![new_path.clone()],
            },
            cx,
        );
        assert_ne!(
            ws.lane_scoped[&ws.active_ref()].git.worktree_refresh,
            crate::workspace::lane_scoped::RefreshSlot::Idle,
            "watcher event must kick git status refresh"
        );
    });
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        let tree = ws.lane_file_tree(id).unwrap();
        let names: Vec<&str> = tree
            .child_entries(tree.root_id)
            .map(|e| e.name.as_str())
            .collect();
        assert!(names.contains(&"c.txt"), "watcher reload must add c.txt");
    });
    let v2: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    assert!(
        !Arc::ptr_eq(&v1, &v2),
        "watcher-driven reload must invalidate the visible cache"
    );

    // The watcher path above already switches the lane to Git and proves a
    // refresh is kicked. Also keep the concurrent-refresh guard covered here
    // instead of paying for another workspace fixture.
    ws.update(cx, |ws, cx| {
        let target = ws.active_ref();
        ws.refresh_git_status(target, cx);
        ws.refresh_git_status(target, cx);
        ws.refresh_git_status(target, cx);
        assert_eq!(
            ws.lane_scoped[&target].git.worktree_refresh,
            crate::workspace::lane_scoped::RefreshSlot::Running { repeat: true }
        );
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        let target = ws.active_ref();
        assert_eq!(
            ws.lane_scoped[&target].git.worktree_refresh,
            crate::workspace::lane_scoped::RefreshSlot::Idle
        );
        assert_eq!(
            ws.lane_scoped[&target].git.tracking_refresh,
            crate::workspace::lane_scoped::RefreshSlot::Idle
        );
    });
}

#[gpui::test]
async fn inactive_lane_event_marks_dirty_then_replays_on_activation(cx: &mut TestAppContext) {
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let active_ref = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(active_ref, cx));
    cx.run_until_parked();
    let active_visible_before: Arc<_> =
        ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(active_ref));

    // Create a sibling lane (inactive). The dirty-replay path triggers a Bulk
    // reload, which scans root + every expanded dir.
    let temp2 = tempfile::tempdir().unwrap();
    let replay_path = temp2.path().join("only-after-activate.txt");
    std::fs::write(&replay_path, b"x").unwrap();
    let inactive_id: daruda_store::project::LaneId = active_ref.lane + 1;
    let inactive_ref = daruda_store::project::LaneRef {
        project: active_ref.project,
        lane: inactive_id,
    };
    let inactive_path = temp2.path().to_path_buf();
    ws.update(cx, |ws, _cx| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes.push(crate::lane::Lane::default_for_project(
                inactive_id,
                inactive_path.clone(),
            ));
        }
        ws.lane_scoped_mut(inactive_ref).files.tree =
            Some(crate::files::tree::FileTree::new(inactive_path));
    });

    ws.update(cx, |ws, cx| {
        ws.queue_files_event(
            inactive_ref,
            crate::files::watcher::DebouncedEvent::Changed {
                paths: vec![replay_path],
            },
            cx,
        );
    });

    ws.read_with(cx, |ws, _| {
        let tree = ws.lane_file_tree(inactive_ref).unwrap();
        assert!(tree.dirty, "inactive lane must record dirty=true");
        // No reload queue work created.
        let q = ws
            .lane_scoped
            .get(&inactive_ref)
            .and_then(|state| state.files.reload_queue.as_ref());
        assert!(q.is_none_or(|q| !q.is_running_for_test()));
    });

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let target = daruda_store::project::LaneRef {
                project: ws.active_ref().project,
                lane: inactive_id,
            };
            ws.activate_lane(target, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let active_visible_after: Arc<_> =
        ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(active_ref));
    assert!(
        !Arc::ptr_eq(&active_visible_before, &active_visible_after),
        "activate_lane must invalidate the previous lane's visible cache"
    );

    ws.read_with(cx, |ws, _| {
        let tree = ws.lane_file_tree(inactive_ref).unwrap();
        assert!(!tree.dirty, "dirty flag must clear after replay");
        let names: Vec<&str> = tree
            .child_entries(tree.root_id)
            .map(|e| e.name.as_str())
            .collect();
        assert!(
            names.contains(&"only-after-activate.txt"),
            "replay must Bulk-reload the root and pick up new files"
        );
    });
    // Hold temp2 alive until here so the FS reads above succeed.
    drop(temp2);
}

// ---------------- hidden toggle + Alt+ collapse ----------------

#[gpui::test]
async fn toggle_files_show_hidden_filters_dotfiles(cx: &mut TestAppContext) {
    // Add a dotfile so the filter has something to drop.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::write(root.join(".env"), b"x").unwrap();
    std::fs::write(root.join("a.txt"), b"a").unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(&root);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();

    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    ws.update(cx, |ws, _cx| {
        let visible = ws.cached_or_rebuild_visible(id);
        let names: Vec<&str> = visible.iter().map(|v| v.name.as_str()).collect();
        assert!(names.contains(&".env"), "default config shows hidden files");
        assert!(names.contains(&"a.txt"));
    });

    // Flip hidden off → dotfile must disappear from the visible list.
    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    ws.update(cx, |ws, cx| ws.toggle_files_show_hidden(cx));
    ws.update(cx, |ws, _cx| {
        let visible: Arc<_> = ws.cached_or_rebuild_visible(id);
        assert!(
            !Arc::ptr_eq(&v1, &visible),
            "show_hidden toggle must invalidate the cache"
        );
        let names: Vec<&str> = visible.iter().map(|v| v.name.as_str()).collect();
        assert!(!names.contains(&".env"), "hidden=off must drop dotfiles");
        assert!(names.contains(&"a.txt"));
    });
    let _ = temp; // keep alive
}

// ---------------- keyboard navigation ----------------

#[gpui::test]
async fn keyboard_selection_moves_activates_and_collapses(cx: &mut TestAppContext) {
    let (wh, ws, temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();
    let visible_first = ws.update(cx, |ws, _| ws.cached_or_rebuild_visible(id))[0].entry_id;

    // None → first row.
    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.move_files_selection(1, window, cx));
    })
    .unwrap();
    let v2: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    assert!(
        !Arc::ptr_eq(&v1, &v2),
        "moving the keyboard cursor must invalidate the visible cache"
    );
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.file_tree.files_selection, Some(visible_first));
    });

    // Down again → next row.
    let visible = ws.update(cx, |ws, _| ws.cached_or_rebuild_visible(id));
    let second_id = visible[1].entry_id;
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.move_files_selection(1, window, cx));
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.file_tree.files_selection, Some(second_id));
    });

    let a_id = ws.update(cx, |ws, _| {
        ws.cached_or_rebuild_visible(id)
            .iter()
            .find(|v| v.name == "a.txt")
            .unwrap()
            .entry_id
    });
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.file_tree.files_selection = Some(a_id);
            ws.invalidate_visible_files_cache(id);
            ws.activate_files_selection(window, cx);
        });
    })
    .unwrap();

    ws.read_with(cx, |ws, _| {
        let fv = ws.focused_file_view().expect("viewer open");
        assert_eq!(fv.path, temp.path().join("a.txt"));
    });

    let sub_id = child_id_by_name(&ws, cx, "sub");
    ws.update(cx, |ws, cx| {
        ws.toggle_files_expand(id, sub_id, cx);
        ws.file_tree.files_selection = Some(sub_id);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(ws.lane_file_tree(id).unwrap().is_expanded(sub_id));
    });

    ws.update(cx, |ws, cx| ws.collapse_at_files_selection(cx));
    ws.read_with(cx, |ws, _| {
        assert!(!ws.lane_file_tree(id).unwrap().is_expanded(sub_id));
    });

    ws.update(cx, |ws, cx| ws.toggle_files_expand(id, sub_id, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(ws.lane_file_tree(id).unwrap().is_expanded(sub_id));
    });

    ws.update(cx, |ws, cx| ws.collapse_files_subtree(id, sub_id, cx));
    ws.read_with(cx, |ws, _| {
        assert!(
            !ws.lane_file_tree(id).unwrap().is_expanded(sub_id),
            "Alt+click collapse must drop the dir from expanded"
        );
    });
}

// ---------------- Lifecycle / cleanup ----------------

#[gpui::test]
async fn finalize_remove_lane_clears_per_lane_state(cx: &mut TestAppContext) {
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let active_ref = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(active_ref, cx));
    cx.run_until_parked();

    // Add a removable git worktree alongside the default. We need
    // a Git kind because validate_remove_lane refuses defaults.
    let removable_id: daruda_store::project::LaneId = active_ref.lane + 1;
    let removable_ref = daruda_store::project::LaneRef {
        project: active_ref.project,
        lane: removable_id,
    };
    let removable_path = ws.read_with(cx, |ws, _| ws.active_lanes()[0].path.clone());
    ws.update(cx, |ws, cx| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes.push(crate::lane::Lane::git(
                removable_id,
                removable_path.clone(),
                Some("feat".into()),
                removable_path.clone(),
                removable_path.clone(),
                1,
            ));
        }
        ws.ensure_file_tree(removable_ref, cx);
    });
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        assert!(ws.lane_file_tree(removable_ref).is_some());
        assert!(
            ws.lane_scoped
                .get(&removable_ref)
                .is_some_and(|state| state.files.watcher.is_some())
        );
        assert!(
            ws.lane_scoped
                .get(&removable_ref)
                .is_some_and(|state| state.files.gitignore.is_some())
        );
    });

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.finalize_remove_lane(removable_ref, window, cx);
        });
    })
    .unwrap();

    ws.read_with(cx, |ws, _| {
        assert!(ws.lane_file_tree(removable_ref).is_none());
        assert!(
            ws.lane_scoped
                .get(&removable_ref)
                .is_none_or(|state| state.files.watcher.is_none())
        );
        assert!(
            ws.lane_scoped
                .get(&removable_ref)
                .is_none_or(|state| state.files.gitignore.is_none())
        );
        assert!(
            ws.lane_scoped
                .get(&removable_ref)
                .is_none_or(|state| state.files.reload_queue.is_none())
        );
        assert!(
            ws.lane_scoped
                .get(&removable_ref)
                .is_none_or(|state| state.files.visible_cache.is_none())
        );
        assert!(!ws.lane_scoped.contains_key(&removable_ref));
    });
}

// ---------------- gitignore ----------------

#[gpui::test]
async fn gitignore_marks_entries_and_rebuilds_matcher_on_change(cx: &mut TestAppContext) {
    // Build a temp project with a .gitignore that ignores `target/`.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::create_dir(root.join("target")).unwrap();
    std::fs::write(root.join("a.txt"), b"a").unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(&root);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();

    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    ws.update(cx, |ws, _cx| {
        let visible = ws.cached_or_rebuild_visible(id);
        let target = visible
            .iter()
            .find(|v| v.name == "target")
            .expect("target dir visible");
        assert!(target.is_ignored, "target/ must be flagged ignored");
        let a = visible
            .iter()
            .find(|v| v.name == "a.txt")
            .expect("a.txt visible");
        assert!(!a.is_ignored, "a.txt must not be flagged ignored");
    });

    std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    let gitignore_path = root.join(".gitignore");
    ws.update(cx, |ws, _cx| {
        ws.queue_files_event(
            id,
            crate::files::watcher::DebouncedEvent::Changed {
                paths: vec![gitignore_path.clone()],
            },
            _cx,
        );
    });
    cx.run_until_parked();
    ws.update(cx, |ws, _cx| {
        let visible = ws.cached_or_rebuild_visible(id);
        let target = visible.iter().find(|v| v.name == "target").unwrap();
        assert!(
            !target.is_ignored,
            "target/ must become unignored after .gitignore reload"
        );
    });

    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    ws.update(cx, |ws, cx| {
        ws.queue_files_event(
            id,
            crate::files::watcher::DebouncedEvent::Changed {
                paths: vec![gitignore_path.clone()],
            },
            cx,
        );
    });
    cx.run_until_parked();

    ws.update(cx, |ws, _cx| {
        let visible = ws.cached_or_rebuild_visible(id);
        let target = visible.iter().find(|v| v.name == "target").unwrap();
        assert!(
            target.is_ignored,
            "target/ must become ignored after .gitignore reload"
        );
    });
    let _ = temp;
}

#[gpui::test]
async fn gitignore_disabled_skips_evaluation(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::create_dir(root.join("target")).unwrap();
    let mut config = daruda_config::Config::default();
    config.left_dock.files_use_gitignore = false;
    let project = daruda_store::project::Project::from_path(&root);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();

    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    ws.update(cx, |ws, _cx| {
        let visible = ws.cached_or_rebuild_visible(id);
        let target = visible.iter().find(|v| v.name == "target").unwrap();
        assert!(
            !target.is_ignored,
            "files_use_gitignore=false must skip evaluation"
        );
    });
    let _ = temp;
}
