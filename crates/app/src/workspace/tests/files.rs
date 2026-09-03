use super::*;
use crate::files::tree::EntryKind;
use std::sync::Arc;

// ================================================================
// Files view — integration + cache invalidation regression tests
// ================================================================

/// Build a Workspace rooted at a fresh tempdir with a few files +
/// one subdirectory. The TempDir stays in scope for the lifetime
/// of the test; dropping it cleans everything up.
fn build_workspace_with_temp_project(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<Workspace>,
    gpui::Entity<Workspace>,
    tempfile::TempDir,
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::write(root.join("a.txt"), b"hello").unwrap();
    std::fs::write(root.join("b.txt"), b"world").unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("sub").join("nested.txt"), b"deep").unwrap();

    crate::test_support::init_gpui_component(cx);
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
    (wh, ws, temp)
}

/// Find the loaded entry id under root with name `name`. Caller
/// must have already driven `ensure_file_tree` + `run_until_parked`.
fn child_id_by_name(
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
    name: &str,
) -> crate::files::tree::EntryId {
    ws.read_with(cx, |ws, _| {
        let id = ws.active_ref();
        let tree = ws.file_tree.file_trees.get(&id).expect("file tree exists");
        for entry in tree.child_entries(tree.root_id) {
            if entry.name == name {
                return entry.id;
            }
        }
        panic!("no child entry named {name:?}");
    })
}

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
        let tree = ws.file_tree.file_trees.get(&id).expect("tree");
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
        let tree = &ws.file_tree.file_trees[&id];
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
        let tree = &ws.file_tree.file_trees[&id];
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
            ws.open_files_entry(id, std::path::PathBuf::from("a.txt"), window, cx);
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
        assert_eq!(fv.lane_id, id.lane);
        assert_eq!(fv.path, std::path::PathBuf::from("a.txt"));
        assert!(!fv.staged, "Files view always uses staged=false");
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
            ws.open_files_entry(id, std::path::PathBuf::from("a.txt"), window, cx);
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
    ws.update(cx, |ws, cx| ws.move_files_selection(1, cx));

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
    let (saved_workspace, saved_projects) = ws
        .read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
        .expect("snapshot_for_disk");
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
        assert_eq!(fv.lane_id, id.lane);
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
        assert!(
            ws.git_status_in_flight.contains(&ws.active_ref()),
            "watcher event must kick git status refresh"
        );
    });
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        let tree = &ws.file_tree.file_trees[&id];
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
        assert!(ws.git_status_in_flight.contains(&target));
        assert!(ws.git_status_pending_repeat.contains(&target));
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        let target = ws.active_ref();
        assert!(!ws.git_status_in_flight.contains(&target));
        assert!(!ws.git_status_pending_repeat.contains(&target));
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
        ws.file_tree.file_trees.insert(
            inactive_ref,
            crate::files::tree::FileTree::new(inactive_path),
        );
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
        let tree = &ws.file_tree.file_trees[&inactive_ref];
        assert!(tree.dirty, "inactive lane must record dirty=true");
        // No reload queue work created.
        let q = ws.file_tree.files_reload_queues.get(&inactive_ref);
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
        let tree = &ws.file_tree.file_trees[&inactive_ref];
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
    ws.update(cx, |ws, cx| ws.move_files_selection(1, cx));
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
    ws.update(cx, |ws, cx| ws.move_files_selection(1, cx));
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
        assert!(ws.file_tree.file_trees[&id].is_expanded(sub_id));
    });

    ws.update(cx, |ws, cx| ws.collapse_at_files_selection(cx));
    ws.read_with(cx, |ws, _| {
        assert!(!ws.file_tree.file_trees[&id].is_expanded(sub_id));
    });

    ws.update(cx, |ws, cx| ws.toggle_files_expand(id, sub_id, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(ws.file_tree.file_trees[&id].is_expanded(sub_id));
    });

    ws.update(cx, |ws, cx| ws.collapse_files_subtree(id, sub_id, cx));
    ws.read_with(cx, |ws, _| {
        assert!(
            !ws.file_tree.file_trees[&id].is_expanded(sub_id),
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
        assert!(ws.file_tree.file_trees.contains_key(&removable_ref));
        assert!(ws.file_tree.file_watchers.contains_key(&removable_ref));
        assert!(
            ws.file_tree
                .files_gitignore_index
                .contains_key(&removable_ref)
        );
    });

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.finalize_remove_lane(removable_ref, window, cx);
        });
    })
    .unwrap();

    ws.read_with(cx, |ws, _| {
        assert!(!ws.file_tree.file_trees.contains_key(&removable_ref));
        assert!(!ws.file_tree.file_watchers.contains_key(&removable_ref));
        assert!(
            !ws.file_tree
                .files_gitignore_index
                .contains_key(&removable_ref)
        );
        assert!(
            !ws.file_tree
                .files_reload_queues
                .contains_key(&removable_ref)
        );
        assert!(
            !ws.file_tree
                .files_visible_cache
                .contains_key(&removable_ref)
        );
        assert!(!ws.git_status_in_flight.contains(&removable_ref));
        assert!(!ws.git_status_pending_repeat.contains(&removable_ref));
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

// ---------------- activate_lane_by_index ----------------

#[gpui::test]
fn activate_lane_by_index_switches_by_tab_order_and_ignores_out_of_range(cx: &mut TestAppContext) {
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    // There is one lane at startup. Add two more with shuffled
    // tab_order so the sort inside activate_lane_by_index is exercised.
    let (_id0, id1, id2) = ws.read_with(cx, |ws, _| {
        let base = ws.active.lane;
        (base, base + 1, base + 2)
    });
    let path = ws.read_with(cx, |ws, _| ws.active_lanes()[0].path.clone());
    ws.update(cx, |ws, _cx| {
        if let Some(p) = ws.active_project_mut() {
            // tab_order 0 → id0 is the existing one. Give it order 2.
            p.lanes[0].tab_order = 2;
            p.lanes.push({
                let mut w = crate::lane::Lane::default_for_project(id1, path.clone());
                w.tab_order = 0;
                w
            });
            p.lanes.push({
                let mut w = crate::lane::Lane::default_for_project(id2, path.clone());
                w.tab_order = 1;
                w
            });
        }
    });

    // Index 0 → tab_order 0 → id1.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_lane_by_index(0, window, cx));
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active.lane, id1, "index 0 should select tab_order 0");
    });

    // Index 1 → tab_order 1 → id2.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_lane_by_index(1, window, cx));
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active.lane, id2, "index 1 should select tab_order 1");
    });

    let initial = ws.read_with(cx, |ws, _| ws.active.lane);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_lane_by_index(99, window, cx));
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active.lane, initial,
            "out-of-range index must be a no-op"
        );
    });
}

// ---------------- Inaccessible lane directory (availability) ----------------

#[gpui::test]
async fn ensure_file_tree_skips_unavailable_lane_and_tears_down_watcher(cx: &mut TestAppContext) {
    use crate::lane::availability::LaneAvailability;

    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());

    // Mark the active lane's root as missing, then request the tree.
    ws.update(cx, |ws, cx| {
        ws.set_lane_availability(id, LaneAvailability::Missing);
        ws.ensure_file_tree(id, cx);
    });
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        assert!(
            !ws.file_tree.file_trees.contains_key(&id),
            "unavailable lane must not get a file tree"
        );
        assert!(
            !ws.file_tree.file_watchers.contains_key(&id),
            "unavailable lane must not get a watcher"
        );
    });

    // First ensure on a present lane spawns the watcher.
    ws.update(cx, |ws, cx| {
        ws.set_lane_availability(id, LaneAvailability::Present);
        ws.ensure_file_tree(id, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(
            ws.file_tree.file_trees.contains_key(&id),
            "present lane gets a file tree"
        );
        assert!(
            ws.file_tree.file_watchers.contains_key(&id),
            "present lane gets a watcher"
        );
    });

    // The lane root vanishes; re-ensuring must remove the watcher.
    ws.update(cx, |ws, cx| {
        ws.set_lane_availability(id, LaneAvailability::Missing);
        ws.ensure_file_tree(id, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(
            !ws.file_tree.file_trees.contains_key(&id),
            "tree torn down once the lane root is missing"
        );
        assert!(
            !ws.file_tree.file_watchers.contains_key(&id),
            "watcher torn down once the lane root is missing"
        );
    });
}

#[gpui::test]
async fn root_and_late_load_errors_update_availability_and_toasts(cx: &mut TestAppContext) {
    use crate::files::tree::{EntryId, FileTree, FileTreeError};
    use crate::lane::availability::LaneAvailability;
    use crate::workspace::left_dock::file_tree_ops::DirLoadSource;
    use daruda_store::observability::error_report::ErrorSeverity;

    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());

    ws.update(cx, |ws, cx| {
        // Seed a tree so `apply_dir_load_result` resolves `is_root`.
        let root = ws.lane_for(id).unwrap().path.clone();
        ws.file_tree
            .file_trees
            .insert(id, FileTree::new(root.clone()));
        // A root load that comes back NotFound is the detection site.
        // `EntryId(0)` is the `root_id` `FileTree::new` assigns to the
        // root node, so passing it here marks this as a root load.
        ws.apply_dir_load_result(
            id,
            EntryId(0),
            Err(FileTreeError::NotFound),
            DirLoadSource::UserExpand,
            cx,
        );

        assert_eq!(
            ws.lane_for(id).unwrap().availability,
            LaneAvailability::Missing,
            "root NotFound must flip availability to Missing"
        );
        let has_error_toast = ws
            .error_toasts(cx)
            .iter()
            .any(|t| t.report.severity == ErrorSeverity::Error);
        assert!(
            !has_error_toast,
            "root read failure must no longer emit an Error toast"
        );

        // Seed a tree so `apply_dir_load_result` resolves `is_root`.
        ws.set_lane_availability(id, LaneAvailability::Present);
        ws.file_tree.file_trees.insert(id, FileTree::new(root));
        // A transient/unknown I/O failure on the root must NOT flip the
        // lane (the directory likely still exists) — instead it surfaces
        // as a normal Error toast so a real I/O failure is not swallowed.
        ws.apply_dir_load_result(
            id,
            EntryId(0),
            Err(FileTreeError::Io(std::io::Error::other("boom"))),
            DirLoadSource::UserExpand,
            cx,
        );

        assert_eq!(
            ws.lane_for(id).unwrap().availability,
            LaneAvailability::Present,
            "a transient root I/O error must keep the lane Present (no teardown)"
        );
        assert!(
            ws.file_tree.file_trees.contains_key(&id),
            "a Present lane keeps its tree — no teardown on a transient error"
        );
        let has_error_toast = ws
            .error_toasts(cx)
            .iter()
            .any(|t| t.report.severity == ErrorSeverity::Error);
        assert!(
            has_error_toast,
            "a genuine root I/O failure must surface as an Error toast"
        );
        // Lane is already Missing (its root flipped earlier). Seed a
        // tree whose root_id is EntryId(0) so a non-root parent_id
        // makes this a *child* load, exercising the late-arrival path.
        let root = ws.lane_for(id).unwrap().path.clone();
        ws.file_tree.file_trees.insert(id, FileTree::new(root));
        ws.set_lane_availability(id, LaneAvailability::Missing);

        // A child load (parent_id != root_id) that lands after the lane
        // went missing — an in-flight read arriving post-teardown.
        ws.apply_dir_load_result(
            id,
            EntryId(999),
            Err(FileTreeError::Io(std::io::Error::other("boom"))),
            DirLoadSource::UserExpand,
            cx,
        );

        let has_warning = ws
            .error_toasts(cx)
            .iter()
            .any(|t| t.report.severity == ErrorSeverity::Warning);
        assert!(
            !has_warning,
            "a late child load on an already-missing lane must not toast"
        );
    });
}

#[gpui::test]
async fn mid_session_root_vanish_tears_down_tree_and_reconciles_project(cx: &mut TestAppContext) {
    use crate::files::tree::{EntryId, FileTreeError};
    use crate::lane::availability::LaneAvailability;
    use crate::workspace::left_dock::file_tree_ops::DirLoadSource;

    let (_wh, ws, temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());

    // An active, Present lane with a live tree + watcher (the case
    // `ensure_file_tree`'s teardown never reaches once a tree exists).
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(ws.file_tree.file_trees.contains_key(&id), "tree exists");
        assert!(
            ws.file_tree.file_watchers.contains_key(&id),
            "watcher exists"
        );
        assert_eq!(
            ws.lane_for(id).unwrap().availability,
            LaneAvailability::Present
        );
        assert_eq!(
            ws.project_for(id.project).unwrap().availability,
            LaneAvailability::Present
        );
    });

    // The lane root (== project root for the default lane) vanishes;
    // the watcher's next bulk reload comes back NotFound on the root.
    drop(temp);
    ws.update(cx, |ws, cx| {
        ws.apply_dir_load_result(
            id,
            EntryId(0),
            Err(FileTreeError::NotFound),
            DirLoadSource::WatcherReload,
            cx,
        );
    });

    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.lane_for(id).unwrap().availability,
            LaneAvailability::Missing,
            "root NotFound flips the lane non-Present"
        );
        assert!(
            !ws.file_tree.file_trees.contains_key(&id),
            "teardown removes the stale tree"
        );
        assert!(
            !ws.file_tree.file_watchers.contains_key(&id),
            "teardown removes the watcher so it stops firing reload spam"
        );
        assert_eq!(
            ws.project_for(id.project).unwrap().availability,
            LaneAvailability::Missing,
            "owning project availability is reconciled off the dead root"
        );
    });
}

#[gpui::test]
async fn raw_file_load_feeds_editor_text(cx: &mut TestAppContext) {
    // Regression: a LaneId/LaneRef merge dropped the load-completion
    // `set_value` into the raw editor, so every non-markdown file
    // opened empty. The editor must hold the file text, and the
    // saved-text baseline must match so a freshly opened file is not
    // dirty.
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    // The test constructor skips WindowRegistry registration; the
    // load-completion handler needs it to find the owning window.
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_files_entry(id, std::path::PathBuf::from("a.txt"), window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    ws.read_with(cx, |ws, cx| {
        let fc = ws.focused_file_content().expect("viewer open");
        assert!(
            matches!(
                fc.view.content,
                crate::workspace::main_area::file_view_pane::PaneFileContent::LoadedRaw
            ),
            "content should settle to LoadedRaw"
        );
        assert_eq!(
            fc.editor_state.read(cx).text().to_string(),
            "hello",
            "editor must hold the file text after load"
        );
        assert_eq!(fc.saved_text, "hello", "saved baseline matches disk");
    });
}

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

/// Whether `text` has a row equal to `line` (trimmed) — a full-line match,
/// not a substring one, so a hunk header's embedded context annotation
/// (`@@ -2,7 +2,7 @@ line 1`) can't be mistaken for a surviving context row.
fn has_line(text: &str, line: &str) -> bool {
    text.lines().any(|l| l.trim() == line)
}

#[gpui::test]
async fn toggle_hide_unchanged_swaps_diff_context_in_the_toggled_pane(cx: &mut TestAppContext) {
    // Regression: the toolbar's hide-unchanged toggle rebuilds the diff
    // editor's synthetic buffer straight from the row list belonging to the
    // *toggled* pane (`toggle_hide_unchanged_for_pane`, not the old
    // focused-pane-only `toggle_hide_unchanged`) — assert the editor text
    // actually swaps between the context-included and context-stripped row
    // sets on each toggle, and flips back cleanly.
    if !crate::lane::git::has_git() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "daruda@test"]);
    run_git(&root, &["config", "user.name", "daruda"]);
    let lines: Vec<String> = (1..=10).map(|n| format!("line {n}")).collect();
    std::fs::write(root.join("f.txt"), lines.join("\n") + "\n").unwrap();
    run_git(&root, &["add", "f.txt"]);
    run_git(&root, &["commit", "-m", "initial"]);

    // Modify one line in the middle so `git diff` produces a single hunk
    // with unchanged context lines on both sides of the change.
    let mut modified = lines.clone();
    modified[4] = "line 5 CHANGED".to_string();
    std::fs::write(root.join("f.txt"), modified.join("\n") + "\n").unwrap();

    crate::test_support::init_gpui_component(cx);
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
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    // `new_with_project_for_test` seeds a pure-placeholder `Default` lane and
    // skips `reconcile_bootstrapped_lanes` (the async git-discovery upgrade
    // the production constructor runs) to keep the suite fast — opt in
    // explicitly, or `git_repo_root_for` sees a non-git lane and the diff
    // load errors out with "No git repository root".
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();

    let lane_id = ws.read_with(cx, |ws, _| ws.active_ref().lane);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_git_file_diff(
                lane_id,
                std::path::PathBuf::from("f.txt"),
                false,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();

    let pane_id = ws.read_with(cx, |ws, cx| {
        let fc = ws.focused_file_content().expect("diff viewer open");
        if let crate::workspace::main_area::file_view_pane::PaneFileContent::Error(e) =
            &fc.view.content
        {
            panic!("load errored: {e}");
        }
        assert!(
            matches!(
                fc.view.content,
                crate::workspace::main_area::file_view_pane::PaneFileContent::LoadedDiff { .. }
            ),
            "content should settle to LoadedDiff, got a different variant"
        );
        let text = fc.editor_state.read(cx).text().to_string();
        assert!(
            has_line(&text, "line 3"),
            "context lines must be visible before toggling hide-unchanged:\n{text}"
        );
        assert!(text.contains("line 5 CHANGED"));
        ws.active_runtime().focused_pane_id
    });

    // Route through `cx.update_window`, same as the toolbar's real mouse-down
    // dispatch (`Window::dispatch_event` checks the window out of `cx.windows`
    // for the duration of the listener) — this is what exposed the regression:
    // `toggle_hide_unchanged_for_pane` used to look up a *fresh* window handle
    // and re-enter `cx.update_window` on it while already nested inside one,
    // which fails ("window not found", silently logged) and left the editor
    // buffer showing the pre-toggle content forever.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.toggle_hide_unchanged_for_pane(pane_id, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    ws.read_with(cx, |ws, cx| {
        let fc = ws.focused_file_content().expect("diff viewer still open");
        assert!(fc.view.hide_unchanged, "toggle must flip hide_unchanged");
        let text = fc.editor_state.read(cx).text().to_string();
        // `git diff`'s hunk header embeds a preceding-context snippet (here it
        // picked "line 1", the plain-text fallback heuristic) that legitimately
        // survives hide-unchanged — check a full *row* line, not a substring,
        // so that annotation can't be mistaken for a surviving context row.
        assert!(
            !has_line(&text, "line 3"),
            "hide-unchanged must drop context lines from the editor buffer:\n{text}"
        );
        assert!(
            text.contains("line 5 CHANGED"),
            "the changed line itself must stay:\n{text}"
        );
    });

    // Toggle back to "show all" — context lines must return.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.toggle_hide_unchanged_for_pane(pane_id, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    ws.read_with(cx, |ws, cx| {
        let fc = ws.focused_file_content().expect("diff viewer still open");
        assert!(!fc.view.hide_unchanged);
        let text = fc.editor_state.read(cx).text().to_string();
        assert!(
            has_line(&text, "line 3"),
            "toggling back to \"show all\" must restore context lines:\n{text}"
        );
    });
}

#[gpui::test]
async fn toggle_hide_unchanged_for_pane_targets_the_clicked_pane_not_the_focused_one(
    cx: &mut TestAppContext,
) {
    // Regression: before `toggle_hide_unchanged_for_pane` threaded a `pane_id`
    // through, the toolbar's hide-unchanged button always mutated
    // `focused_file_content_mut()` — so clicking it on a *non-focused* split
    // pane silently hid context on whichever pane happened to be focused
    // instead. Split a Changes-mode pane away from focus, toggle it by id,
    // and assert only that pane changed.
    if !crate::lane::git::has_git() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "daruda@test"]);
    run_git(&root, &["config", "user.name", "daruda"]);
    let lines: Vec<String> = (1..=10).map(|n| format!("line {n}")).collect();
    std::fs::write(root.join("f.txt"), lines.join("\n") + "\n").unwrap();
    std::fs::write(root.join("g.txt"), b"unrelated file\n").unwrap();
    run_git(&root, &["add", "f.txt", "g.txt"]);
    run_git(&root, &["commit", "-m", "initial"]);

    let mut modified = lines.clone();
    modified[4] = "line 5 CHANGED".to_string();
    std::fs::write(root.join("f.txt"), modified.join("\n") + "\n").unwrap();

    crate::test_support::init_gpui_component(cx);
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
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();

    let lane_id = ws.read_with(cx, |ws, _| ws.active_ref().lane);

    // Pane A: f.txt in Changes mode. Becomes the focused pane.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_git_file_diff(
                lane_id,
                std::path::PathBuf::from("f.txt"),
                false,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
    let pane_a_id = ws.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);

    // Pane B: split g.txt to the right of pane A. `open_file_split_right`
    // focuses the new pane, so pane A is no longer focused.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_file_split_right(
                lane_id,
                std::path::PathBuf::from("g.txt"),
                pane_a_id,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();

    let pane_b_id = ws.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
    assert_ne!(pane_a_id, pane_b_id, "split must focus the new pane");

    // Click pane A's own toolbar toggle while B is focused. Routed through
    // `cx.update_window`, matching the real mouse-down dispatch, so a
    // regression back to a nested fresh-window lookup inside the toggle
    // (see the sibling test above) would fail here too.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.toggle_hide_unchanged_for_pane(pane_a_id, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    ws.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.active_runtime().focused_pane_id,
            pane_b_id,
            "toggling pane A's button must not steal focus from pane B"
        );

        let pane_a = ws
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_a_id)
            .and_then(|p| p.file_content())
            .expect("pane A still open");
        assert!(
            pane_a.view.hide_unchanged,
            "pane A (the clicked pane) must have hide_unchanged flipped"
        );
        let text_a = pane_a.editor_state.read(cx).text().to_string();
        assert!(
            !has_line(&text_a, "line 3"),
            "pane A's editor buffer must have dropped context lines:\n{text_a}"
        );

        let pane_b = ws
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_b_id)
            .and_then(|p| p.file_content())
            .expect("pane B still open");
        assert!(
            !pane_b.view.hide_unchanged,
            "pane B (focused, not clicked) must be untouched by pane A's toggle"
        );
    });
}

#[gpui::test]
#[should_panic(expected = "file pane content targets lane")]
async fn open_pane_file_view_asserts_lane_id_matches_active_lane(cx: &mut TestAppContext) {
    // `open_pane_file_view` always pushes the new pane into
    // `self.active_runtime_mut()`, but stamps the pane's owner/`lane_id` from
    // the caller-supplied `lane_id`. `load_pane_file_content`'s completion
    // callback later looks the pane back up via `runtimes.get_mut(&owner)` —
    // a *different* runtime than the active one the pane actually lives in,
    // if `lane_id` isn't the active lane. That silently drops the load (the
    // pane sticks on "Loading" forever, no error surfaced). The debug assert
    // in `Workspace::active_lane_ref` is the only thing standing between
    // that regression and a green test suite — pin it firing here so a
    // future edit can't drop it unnoticed.
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let active_lane = ws.read_with(cx, |ws, _| ws.active_ref().lane);
    let bogus_lane = active_lane + 1;
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_pane_file_view(
                bogus_lane,
                std::path::PathBuf::from("a.txt"),
                false,
                crate::workspace::main_area::file_view_pane::FileViewMode::Raw,
                window,
                cx,
            );
        });
    })
    .unwrap();
}

/// A workspace over a real git repo holding one committed `f.txt`, with the
/// placeholder lane already upgraded to a Git one (`refresh_git_status` is a
/// no-op otherwise). Returns the repo root plus `f.txt`'s absolute path joined
/// onto the *lane* root — the base every opener uses. On macOS the raw tempdir
/// path is the uncanonical `/var/…` alias of the lane's `/private/var/…`, so
/// joining onto the tempdir instead would never match a pane's path.
#[allow(clippy::type_complexity)]
fn build_committed_git_workspace(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<Workspace>,
    gpui::Entity<Workspace>,
    daruda_store::project::LaneRef,
    std::path::PathBuf,
    std::path::PathBuf,
    tempfile::TempDir,
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "daruda@test"]);
    run_git(&root, &["config", "user.name", "daruda"]);
    std::fs::write(root.join("f.txt"), b"one\n").unwrap();
    run_git(&root, &["add", "f.txt"]);
    run_git(&root, &["commit", "-m", "initial"]);

    crate::test_support::init_gpui_component(cx);
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
    ws.update(cx, |ws, cx| ws.reconcile_bootstrapped_lanes(cx));
    cx.run_until_parked();
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    let abs = ws
        .read_with(cx, |ws, _| ws.lane_for(id).map(|l| l.path.clone()))
        .expect("lane exists")
        .join("f.txt");
    (wh, ws, id, root, abs, temp)
}

/// `file_status` is written once at open time and is deliberately not
/// persisted, so a restored pane starts with `None` and a pane held open
/// across an edit or a commit would otherwise keep whatever the opening click
/// saw — the toolbar's mode strip reads it to decide whether to offer Changes
/// at all. `refresh_git_status` re-derives it via `sync_file_pane_statuses`;
/// this drives the real fetch and pins both directions.
#[gpui::test]
async fn git_status_refresh_re_derives_open_file_panes_status(cx: &mut TestAppContext) {
    if !crate::lane::git::has_git() {
        return;
    }

    let (wh, ws, id, root, abs, _temp) = build_committed_git_workspace(cx);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_files_entry(id, abs.clone(), window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.focused_file_view()
                .expect("file viewer open")
                .file_status,
            None,
            "a restore-shaped open carries no status of its own"
        );
    });

    // Dirty the file, then let a real status fetch land on the open pane.
    std::fs::write(&abs, b"two\n").unwrap();
    ws.update(cx, |ws, cx| ws.refresh_git_status(id, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.focused_file_view()
                .expect("file viewer open")
                .file_status,
            Some('M'),
            "a refreshed status must reach the open pane"
        );
    });

    // Commit it: the pane must drop the badge too, so the toolbar stops
    // offering a Changes view of a file that no longer has one.
    run_git(&root, &["add", "f.txt"]);
    run_git(&root, &["commit", "-m", "second"]);
    ws.update(cx, |ws, cx| ws.refresh_git_status(id, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.focused_file_view()
                .expect("file viewer open")
                .file_status,
            None,
            "a cleared status must clear the pane's badge too"
        );
    });
}

/// The Files view opens a row by click with an absolute path; Enter on the
/// keyboard cursor must produce the same path, or `open_pane_file_view`'s
/// dedupe misses and the same file lands in a second tab.
#[gpui::test]
async fn enter_and_click_open_the_same_files_row_into_one_tab(cx: &mut TestAppContext) {
    let (wh, ws, temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    let a_id = child_id_by_name(&ws, cx, "a.txt");
    ws.update(cx, |ws, _cx| {
        ws.file_tree.files_selection = Some(a_id);
    });
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_files_selection(window, cx));
    })
    .unwrap();

    let abs = temp.path().join("a.txt");
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.focused_file_view().expect("file viewer open").path,
            abs,
            "Enter must open the absolute path, like the row's click handler"
        );
    });
    let after_enter = ws.read_with(cx, |ws, _| ws.active_runtime().tabs.len());

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_files_entry(id, abs, window, cx));
    })
    .unwrap();
    assert_eq!(
        ws.read_with(cx, |ws, _| ws.active_runtime().tabs.len()),
        after_enter,
        "clicking the row Enter already opened must reuse its tab"
    );
}

/// `file_status` is derived inside `open_pane_file_view`, not supplied by the
/// caller: the agent-chat diff header, agent-chat Markdown file links, the
/// skills panel and the task form all open a file with no git context of their
/// own, and once the toolbar's mode strip started gating the Changes segment
/// on `file_status.is_some()` a hardcoded `None` meant a changed file opened
/// that way offered no diff at all. Also covers the dedupe path, which returns
/// early on an existing tab and so has to re-stamp it rather than leave the
/// status frozen at whatever the first open saw.
#[gpui::test]
async fn opening_a_changed_file_without_git_context_still_resolves_its_status(
    cx: &mut TestAppContext,
) {
    use crate::workspace::main_area::file_view_pane::FileViewMode;

    if !crate::lane::git::has_git() {
        return;
    }
    let (wh, ws, id, _root, abs, _temp) = build_committed_git_workspace(cx);

    std::fs::write(&abs, b"two\n").unwrap();
    ws.update(cx, |ws, cx| ws.refresh_git_status(id, cx));
    cx.run_until_parked();

    // The shape every context-free opener uses (agent chat / skills / tasks).
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_pane_file_view(id.lane, abs.clone(), false, FileViewMode::Raw, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.focused_file_view()
                .expect("file viewer open")
                .file_status,
            Some('M'),
            "a context-free open must still resolve the file's status"
        );
    });

    // Re-opening lands in the dedupe branch. Move the cache out from under the
    // pane first, so only a re-stamp there can bring it back in line.
    ws.update(cx, |ws, _cx| {
        ws.git_status_cache
            .insert(id, crate::lane::git::GitStatusData::default());
    });
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_pane_file_view(id.lane, abs.clone(), false, FileViewMode::Raw, window, cx);
        });
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.focused_file_view()
                .expect("file viewer open")
                .file_status,
            None,
            "reusing an existing tab must re-stamp it from the current cache"
        );
    });
}

/// Installing new content must release the GPU images the previous content's
/// slots indexed. `RenderImage` has no `Drop` and the Metal atlas never
/// evicts, so a table that is replaced without a release leaks its textures
/// for the process's lifetime — the defect `install_content` exists to close.
#[gpui::test]
async fn installing_content_releases_the_previous_image_table(cx: &mut TestAppContext) {
    use crate::workspace::main_area::file_view_pane::PaneFileContent;
    use crate::workspace::main_area::file_view_pane::images::MdImages;
    use crate::workspace::main_area::file_view_pane::visual::RasterImage;

    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_files_entry(id, std::path::PathBuf::from("a.txt"), window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    let pane_id = ws.read_with(cx, |ws, _| {
        ws.active_runtime()
            .panes
            .iter()
            .find(|p| p.file_view().is_some())
            .expect("a file pane is open")
            .id
    });

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let fc = ws
                .active_runtime_mut()
                .panes
                .iter_mut()
                .find(|p| p.id == pane_id)
                .and_then(|p| p.file_content_mut())
                .expect("file content");
            *fc.images_for_test() = MdImages::from_rasters(vec![Some(RasterImage {
                width: 1,
                height: 1,
                bgra: vec![0, 0, 0, 255],
                scale: 1.0,
            })]);
            assert!(
                fc.images_for_test().get(0).is_some(),
                "seeded table has slot 0"
            );

            fc.install_content(PaneFileContent::LoadedRaw, Vec::new(), Some(window), cx);
            assert!(
                fc.images_for_test().get(0).is_none(),
                "install_content must release the previous table, not leave it behind"
            );
        });
    })
    .unwrap();
}
