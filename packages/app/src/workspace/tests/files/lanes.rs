use super::*;

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
            ws.lane_file_tree(id).is_none(),
            "unavailable lane must not get a file tree"
        );
        assert!(
            ws.lane_scoped
                .get(&id)
                .is_none_or(|state| state.files.watcher.is_none()),
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
            ws.lane_file_tree(id).is_some(),
            "present lane gets a file tree"
        );
        assert!(
            ws.lane_scoped
                .get(&id)
                .is_some_and(|state| state.files.watcher.is_some()),
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
            ws.lane_file_tree(id).is_none(),
            "tree torn down once the lane root is missing"
        );
        assert!(
            ws.lane_scoped
                .get(&id)
                .is_none_or(|state| state.files.watcher.is_none()),
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
        ws.lane_scoped_mut(id).files.tree = Some(FileTree::new(root.clone()));
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
        ws.lane_scoped_mut(id).files.tree = Some(FileTree::new(root));
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
            ws.lane_file_tree(id).is_some(),
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
        ws.lane_scoped_mut(id).files.tree = Some(FileTree::new(root));
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
        assert!(ws.lane_file_tree(id).is_some(), "tree exists");
        assert!(
            ws.lane_scoped
                .get(&id)
                .is_some_and(|state| state.files.watcher.is_some()),
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
            ws.lane_file_tree(id).is_none(),
            "teardown removes the stale tree"
        );
        assert!(
            ws.lane_scoped
                .get(&id)
                .is_none_or(|state| state.files.watcher.is_none()),
            "teardown removes the watcher so it stops firing reload spam"
        );
        assert_eq!(
            ws.project_for(id.project).unwrap().availability,
            LaneAvailability::Missing,
            "owning project availability is reconciled off the dead root"
        );
    });
}
