use super::*;
use crate::files::tree::EntryKind;
use std::sync::Arc;

// ================================================================
// W-7 Files view — integration + cache invalidation regression tests
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
async fn ensure_file_tree_loads_root_children(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    ws.update(cx, |ws, cx| {
        let id = ws.active_ref();
        ws.ensure_file_tree(id, cx);
    });
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        let id = ws.active_ref();
        let tree = ws.file_tree.file_trees.get(&id).expect("tree");
        let names: Vec<&str> = tree
            .child_entries(tree.root_id)
            .map(|e| e.name.as_str())
            .collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
        assert!(names.contains(&"sub"));
    });
}

#[gpui::test]
async fn toggle_dir_lazy_loads_children(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    // `sub` has no children loaded yet (UnloadedDir).
    let sub_id = child_id_by_name(&ws, cx, "sub");
    ws.read_with(cx, |ws, _| {
        let tree = &ws.file_tree.file_trees[&id];
        let entry = tree.entry(sub_id).unwrap();
        assert_eq!(entry.kind, EntryKind::UnloadedDir);
        assert!(tree.child_ids(sub_id).is_empty());
    });

    // Toggle expand → kicks load.
    ws.update(cx, |ws, cx| ws.toggle_files_expand(id, sub_id, cx));
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
async fn clicking_file_opens_file_viewer_in_raw_mode(cx: &mut TestAppContext) {
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
}

#[gpui::test]
async fn save_restore_preserves_file_viewer_pane(cx: &mut TestAppContext) {
    // End-to-end: open a file → save_state → fresh workspace →
    // restore_state → file pane comes back with the same path,
    // lane, and view mode.
    let (wh, ws, temp) = build_workspace_with_temp_project(cx);
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

    // Capture the current state — the file pane is now part of the
    // active lane's tab list.
    let (saved_workspace, saved_projects) = ws
        .read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
        .expect("snapshot_for_disk");
    assert_eq!(saved_projects.len(), 1);
    assert_eq!(saved_projects[0].lanes.len(), 1);
    let saved_tabs = &saved_projects[0].lanes[0].tabs;
    assert!(
        saved_tabs.iter().any(|t| matches!(
            &t.layout,
            daruda_store::project::SerializedLayout::Leaf { file: Some(fc), .. }
                if fc.path == std::path::Path::new("a.txt")
        )),
        "saved state must include a Leaf with file: Some(..) for a.txt",
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

#[gpui::test]
async fn clicking_same_file_again_activates_existing_tab(cx: &mut TestAppContext) {
    // Clicking the same file twice activates the existing tab
    // (dedupe). Closing the viewer goes through Cmd+W; re-clicks
    // never close.
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    let initial_tab_count = ws.read_with(cx, |ws, _| ws.active_runtime().tabs.len());

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_files_entry(id, std::path::PathBuf::from("a.txt"), window, cx);
        });
    })
    .unwrap();
    let after_first = ws.read_with(cx, |ws, _| ws.active_runtime().tabs.len());
    assert_eq!(
        after_first,
        initial_tab_count + 1,
        "first click adds a file-viewer tab"
    );

    // Re-click same file → activates existing tab, no new tab added.
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
}

#[gpui::test]
async fn cached_visible_lists_root_children_after_load(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    ws.update(cx, |ws, _cx| {
        let visible = ws.cached_or_rebuild_visible(id);
        let names: Vec<&str> = visible.iter().map(|v| v.name.as_str()).collect();
        // Sort puts dirs first ("sub"), then alpha files.
        assert_eq!(names, vec!["sub", "a.txt", "b.txt"]);
    });
}

#[gpui::test]
async fn open_files_entry_opens_file_viewer(cx: &mut TestAppContext) {
    // Left-dock selection follows the keyboard cursor (driven by
    // `files_selection`), not the focused file pane. Opening a
    // file therefore should populate the viewer regardless of
    // which row currently shows the selected background.
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
    ws.read_with(cx, |ws, _| {
        let fv = ws.focused_file_view().expect("viewer open");
        assert_eq!(fv.path, std::path::PathBuf::from("a.txt"));
    });
}

#[gpui::test]
async fn keyboard_move_after_mouse_open_clears_old_highlight(cx: &mut TestAppContext) {
    // Reproduces: mouse-click highlight stayed on the old file
    // after the user pressed an arrow key. The fix: dock
    // background follows `files_selection` only, so moving the
    // cursor moves the bg with it.
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    let a_id = ws.update(cx, |ws, _| {
        ws.cached_or_rebuild_visible(id)
            .iter()
            .find(|v| v.name == "a.txt")
            .unwrap()
            .entry_id
    });

    // Simulate mouse click on a.txt: viewer opens, cursor on a.txt.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.file_tree.files_selection = Some(a_id);
            ws.invalidate_visible_files_cache(id);
            ws.open_files_entry(id, std::path::PathBuf::from("a.txt"), window, cx);
        });
    })
    .unwrap();

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
}

// ---------------- Cache invalidation regression ----------------

#[gpui::test]
async fn cache_invalidates_on_expand_toggle(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();
    let sub_id = child_id_by_name(&ws, cx, "sub");

    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    ws.update(cx, |ws, cx| ws.toggle_files_expand(id, sub_id, cx));
    // Cache should invalidate even before the load returns; the
    // expanded set itself changed.
    let v2: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    assert!(
        !Arc::ptr_eq(&v1, &v2),
        "toggle_files_expand must invalidate the visible cache"
    );
}

#[gpui::test]
async fn cache_invalidates_on_dir_load_result(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    // Snapshot before the root scan returns.
    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    cx.run_until_parked();
    // Loading the root flips PendingDir → Dir + adds children.
    let v2: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    assert!(
        !Arc::ptr_eq(&v1, &v2),
        "apply_dir_load_result must invalidate the visible cache"
    );
}

#[gpui::test]
async fn cache_invalidates_on_focused_file_change(cx: &mut TestAppContext) {
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
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
}

#[gpui::test]
async fn cache_invalidates_on_active_lane_change(cx: &mut TestAppContext) {
    // Build a workspace, then add a second lane manually so
    // we can `activate_lane` between them. Building a real
    // multi-lane setup needs git plumbing; simulating one via
    // direct `lanes.push` is enough for the cache check.
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let original_ref = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(original_ref, cx));
    cx.run_until_parked();

    // Add a second lane pointing at the same temp dir; for
    // cache invalidation the path content does not matter.
    let second_id: daruda_store::project::LaneId = original_ref.lane + 1;
    let new_path = ws.read_with(cx, |ws, _| ws.active_lanes()[0].path.clone());
    ws.update(cx, |ws, cx| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes
                .push(crate::lane::Lane::default_for_project(second_id, new_path));
        }
        // Prime the visible cache for the original.
        let _ = ws.cached_or_rebuild_visible(original_ref);
        cx.notify();
    });

    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(original_ref));
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let target = daruda_store::project::LaneRef {
                project: ws.active_ref().project,
                lane: second_id,
            };
            ws.activate_lane(target, window, cx)
        });
    })
    .unwrap();
    let v2: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(original_ref));
    assert!(
        !Arc::ptr_eq(&v1, &v2),
        "activate_lane must invalidate the previous lane's visible cache"
    );
}

#[gpui::test]
async fn cache_invalidates_on_git_status_update(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    // refresh_git_status is the production caller, but it short-
    // circuits for non-git Default lanes. Simulate the result
    // path directly to drive trigger #6 — git_status_cache write
    // followed by invalidation.
    ws.update(cx, |ws, cx| {
        let active_ref = ws.active_ref();
        ws.git_status_cache.insert(
            active_ref,
            crate::lane::git::GitStatusData {
                staged: vec![crate::lane::git::GitFileEntry {
                    x: 'M',
                    y: ' ',
                    path: std::path::PathBuf::from("a.txt"),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        ws.invalidate_visible_files_cache(id);
        cx.notify();
    });
    let v2: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    assert!(
        !Arc::ptr_eq(&v1, &v2),
        "git_status_cache change must invalidate the visible cache"
    );
    // And the badge must be populated.
    ws.update(cx, |ws, _cx| {
        let visible = ws.cached_or_rebuild_visible(id);
        let a = visible.iter().find(|v| v.name == "a.txt").expect("a.txt");
        assert_eq!(a.git_status, Some('M'));
    });
}

// ---------------- W-7g watcher / dirty flag ----------------

#[gpui::test]
async fn watcher_event_creates_entry_in_expanded_dir(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

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
}

#[gpui::test]
async fn inactive_lane_event_marks_dirty_only(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let active_ref = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(active_ref, cx));
    cx.run_until_parked();

    // Create a sibling lane (inactive). For the dirty check we
    // do not need a real second project root; we just need a
    // separate FileTree entry.
    let inactive_id: daruda_store::project::LaneId = active_ref.lane + 1;
    let inactive_ref = daruda_store::project::LaneRef {
        project: active_ref.project,
        lane: inactive_id,
    };
    let inactive_path = ws.read_with(cx, |ws, _| ws.active_lanes()[0].path.clone());
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
                paths: vec![std::path::PathBuf::from("/tmp/whatever")],
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
}

#[gpui::test]
async fn activate_lane_replays_dirty_with_bulk_reload(cx: &mut TestAppContext) {
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let active_ref = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(active_ref, cx));
    cx.run_until_parked();

    // Build a second lane pointing at a fresh tempdir whose
    // contents we can mutate. The dirty-replay path triggers a
    // Bulk reload, which scans root + every expanded dir.
    let temp2 = tempfile::tempdir().unwrap();
    std::fs::write(temp2.path().join("only-after-activate.txt"), b"x").unwrap();
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
            crate::files::tree::FileTree::new(inactive_path.clone()),
        );
        // Mark dirty as if a watcher event had fired while inactive.
        ws.file_tree
            .file_trees
            .get_mut(&inactive_ref)
            .unwrap()
            .dirty = true;
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

#[gpui::test]
async fn refresh_git_status_collapses_concurrent_calls(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let _id = ws.read_with(cx, |ws, _| ws.active_ref());
    // Default lane (non-git) → refresh_git_status returns
    // before touching the in-flight guard. Switch the lane's
    // kind so the guard path actually runs.
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

    // First call enters the guard. Second + third calls during
    // the same update cycle hit the "already in flight" branch
    // and request a single repeat.
    ws.update(cx, |ws, cx| {
        let target = ws.active_ref();
        ws.refresh_git_status(target, cx);
        ws.refresh_git_status(target, cx);
        ws.refresh_git_status(target, cx);
        assert!(ws.git_status_in_flight.contains(&target));
        assert!(ws.git_status_pending_repeat.contains(&target));
    });
    cx.run_until_parked();
    // After the in-flight task completes, the repeat fires and
    // also completes — both flags settle to clear.
    ws.read_with(cx, |ws, _| {
        let target = ws.active_ref();
        assert!(!ws.git_status_in_flight.contains(&target));
        assert!(!ws.git_status_pending_repeat.contains(&target));
    });
}

#[gpui::test]
async fn watcher_event_triggers_git_status_refresh(cx: &mut TestAppContext) {
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

    // Inject a watcher event; queue_files_event must fan out to
    // refresh_git_status as well as the tree reload.
    let new_path = ws.read_with(cx, |ws, _| ws.active_lanes()[0].path.join("e.txt"));
    std::fs::write(&new_path, b"e").unwrap();
    ws.update(cx, |ws, cx| {
        ws.queue_files_event(
            id,
            crate::files::watcher::DebouncedEvent::Changed {
                paths: vec![new_path.clone()],
            },
            cx,
        );
        // Marker that the in-flight guard was touched (i.e.
        // refresh_git_status actually ran rather than no-opping).
        assert!(
            ws.git_status_in_flight.contains(&ws.active_ref()),
            "watcher event must kick git status refresh"
        );
    });
    cx.run_until_parked();
}

// ---------------- W-7h hidden toggle + Alt+ collapse ----------------

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
    ws.update(cx, |ws, cx| ws.toggle_files_show_hidden(cx));
    ws.update(cx, |ws, _cx| {
        let visible = ws.cached_or_rebuild_visible(id);
        let names: Vec<&str> = visible.iter().map(|v| v.name.as_str()).collect();
        assert!(!names.contains(&".env"), "hidden=off must drop dotfiles");
        assert!(names.contains(&"a.txt"));
    });
    let _ = temp; // keep alive
}

#[gpui::test]
async fn cache_invalidates_on_show_hidden_toggle(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    ws.update(cx, |ws, cx| ws.toggle_files_show_hidden(cx));
    let v2: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    assert!(
        !Arc::ptr_eq(&v1, &v2),
        "show_hidden toggle (trigger #7) must invalidate the cache"
    );
}

#[gpui::test]
async fn alt_click_collapse_drops_descendants_from_expanded(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    // Expand `sub/` so it has loaded children, then expand `sub`'s
    // child file row implicitly via toggle (files do not actually
    // expand; we just need the parent + a child dir for the test).
    // The temp project has `sub/nested.txt` (a file), so just
    // expanding `sub` is enough — the recursive collapse will drop
    // `sub` from expanded.
    let sub_id = child_id_by_name(&ws, cx, "sub");
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

// ---------------- W-7i keyboard navigation ----------------

#[gpui::test]
async fn move_files_selection_advances_through_visible(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();
    let visible_first = ws.update(cx, |ws, _| ws.cached_or_rebuild_visible(id))[0].entry_id;

    // None → first row.
    ws.update(cx, |ws, cx| ws.move_files_selection(1, cx));
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
}

#[gpui::test]
async fn files_activate_opens_file_at_cursor(cx: &mut TestAppContext) {
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    // Park cursor on a.txt explicitly.
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
        assert_eq!(fv.path, std::path::PathBuf::from("a.txt"));
    });
}

#[gpui::test]
async fn files_collapse_at_cursor_drops_expand(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

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
}

#[gpui::test]
async fn cache_invalidates_on_keyboard_selection_move(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    ws.update(cx, |ws, cx| ws.move_files_selection(1, cx));
    let v2: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    assert!(
        !Arc::ptr_eq(&v1, &v2),
        "moving the keyboard cursor must invalidate the visible cache"
    );
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

// ---------------- W-7j gitignore ----------------

#[gpui::test]
async fn gitignore_marks_target_dir_entries_ignored(cx: &mut TestAppContext) {
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

#[gpui::test]
async fn gitignore_change_event_rebuilds_matcher(cx: &mut TestAppContext) {
    // Start with a .gitignore that does NOT cover `target/`.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    std::fs::create_dir(root.join("target")).unwrap();
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
        let target = visible.iter().find(|v| v.name == "target").unwrap();
        assert!(!target.is_ignored, "target/ initially unignored");
    });

    // Mutate .gitignore to add target/, then inject a watcher event
    // pointing at the .gitignore file itself.
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    let gitignore_path = root.join(".gitignore");
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
async fn cache_invalidates_on_watcher_event(cx: &mut TestAppContext) {
    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();
    let v1: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));

    // Inject a watcher event; the chain queue → kick → load_dir →
    // apply_dir_load_result invalidates the cache at the end.
    let new_path = ws.read_with(cx, |ws, _| ws.active_lanes()[0].path.join("d.txt"));
    std::fs::write(&new_path, b"d").unwrap();
    ws.update(cx, |ws, cx| {
        ws.queue_files_event(
            id,
            crate::files::watcher::DebouncedEvent::Changed {
                paths: vec![new_path.clone()],
            },
            cx,
        );
    });
    cx.run_until_parked();

    let v2: Arc<_> = ws.update(cx, |ws, _cx| ws.cached_or_rebuild_visible(id));
    assert!(
        !Arc::ptr_eq(&v1, &v2),
        "watcher-driven reload must invalidate the visible cache"
    );
}

// ---------------- W-9 activate_lane_by_index ----------------

#[gpui::test]
fn activate_lane_by_index_switches_to_nth_by_tab_order(cx: &mut TestAppContext) {
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
}

#[gpui::test]
fn activate_lane_by_index_out_of_range_is_noop(cx: &mut TestAppContext) {
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
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
async fn ensure_file_tree_skips_unavailable_lane(cx: &mut TestAppContext) {
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
}

#[gpui::test]
async fn ensure_file_tree_tears_down_watcher_when_lane_goes_missing(cx: &mut TestAppContext) {
    use crate::lane::availability::LaneAvailability;

    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());

    // First ensure on a present lane spawns the watcher.
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
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
            !ws.file_tree.file_watchers.contains_key(&id),
            "watcher torn down once the lane root is missing"
        );
    });
}

#[gpui::test]
async fn root_load_error_flips_availability_without_error_toast(cx: &mut TestAppContext) {
    use crate::files::tree::{EntryId, FileTree, FileTreeError};
    use crate::lane::availability::LaneAvailability;
    use crate::workspace::left_dock::file_tree_ops::DirLoadSource;
    use daruda_store::observability::error_report::ErrorSeverity;

    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());

    ws.update(cx, |ws, cx| {
        // Seed a tree so `apply_dir_load_result` resolves `is_root`.
        let root = ws.lane_for(id).unwrap().path.clone();
        ws.file_tree.file_trees.insert(id, FileTree::new(root));
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
    });
}

#[gpui::test]
async fn root_load_transient_io_stays_present_and_emits_error_toast(cx: &mut TestAppContext) {
    use crate::files::tree::{EntryId, FileTree, FileTreeError};
    use crate::lane::availability::LaneAvailability;
    use crate::workspace::left_dock::file_tree_ops::DirLoadSource;
    use daruda_store::observability::error_report::ErrorSeverity;

    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());

    ws.update(cx, |ws, cx| {
        // Seed a tree so `apply_dir_load_result` resolves `is_root`.
        let root = ws.lane_for(id).unwrap().path.clone();
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
async fn reactivated_missing_lane_with_tabs_selects_empty_state(cx: &mut TestAppContext) {
    use crate::lane::availability::LaneAvailability;

    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());

    // A reactivated lane whose runtime was frozen with tabs still
    // populated, but whose root is now Missing. The render gate keys
    // off availability FIRST, so it must pick the empty-state even
    // though `tabs` is non-empty. Assert the gating predicate on
    // Workspace state (a direct render assertion is impractical here).
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_tab(window, cx); // simulate the frozen, still-open tab
            ws.set_lane_availability(id, LaneAvailability::Missing);
        });
    })
    .unwrap();

    // Assert the two post-conditions that together force the
    // empty-state branch — non-Present availability AND non-empty tabs
    // (the frozen-runtime case) — rather than re-deriving the render
    // gate's predicate, so the test survives a predicate refactor.
    ws.read_with(cx, |ws, _| {
        assert!(
            !ws.active_runtime().tabs.is_empty(),
            "active lane still has frozen tabs"
        );
        assert_eq!(
            ws.active_lane().map(|l| l.availability),
            Some(LaneAvailability::Missing),
            "active lane is non-Present, so the render gate selects the \
             empty-state regardless of the stale tabs above"
        );
    });
}

#[gpui::test]
async fn child_load_error_on_already_missing_lane_emits_no_warning(cx: &mut TestAppContext) {
    use crate::files::tree::{EntryId, FileTree, FileTreeError};
    use crate::lane::availability::LaneAvailability;
    use crate::workspace::left_dock::file_tree_ops::DirLoadSource;
    use daruda_store::observability::error_report::ErrorSeverity;

    let (_wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());

    ws.update(cx, |ws, cx| {
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
async fn raw_file_load_feeds_editor_text(cx: &mut TestAppContext) {
    // Regression: the LaneId/LaneRef merge (73de52a) dropped the
    // load-completion `set_value` into the raw editor, so every
    // non-markdown file opened as an empty editor. The editor must
    // hold the file text (and the saved-text baseline must match,
    // so a freshly opened file is not dirty).
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
