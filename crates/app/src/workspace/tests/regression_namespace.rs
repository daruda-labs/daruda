//! Fan-out regression: confirm `persist_state` writes one workspace
//! file (not one per project) and one project file per project.
//! Includes an io.whatap-style scenario: two workspaces holding the
//! same project root should not pollute each other's snapshot.

use super::*;

#[gpui::test]
fn persist_state_writes_exactly_one_workspace_file(cx: &mut TestAppContext) {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().to_path_buf();

    // Project A roots (use disposable directories under /tmp). The
    // lane bootstrap inspects the path with `fs::metadata`, but
    // missing dirs degrade gracefully to a `Default` lane — fine
    // for persistence shape verification.
    let root_a = std::env::temp_dir().join("daruda_regression_ns_a");
    let root_b = std::env::temp_dir().join("daruda_regression_ns_b");
    let _ = std::fs::create_dir_all(&root_a);
    let _ = std::fs::create_dir_all(&root_b);

    let config = daruda_config::Config::default();
    let project_a = daruda_store::project::Project::from_path(&root_a);

    // First project supplied via `new_with_project`; second added via
    // `add_project` so the workspace owns two runtime projects.
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(&config, Some(project_a), data_dir.clone(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_project(root_b.clone(), window, cx);
        })
    })
    .unwrap();

    // `add_project` schedules persist via `mutate_durable` → `cx.defer`.
    // Force the defer to drain by reading + running an explicit persist
    // synchronously on the workspace entity so the assertions see the
    // post-add-project disk shape.
    ws.read_with(cx, |w, cx| w.persist_state(cx));

    let workspaces_dir = daruda_store::project::workspaces_dir_in(&data_dir);
    let workspace_count = std::fs::read_dir(&workspaces_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|s| s == "json").unwrap_or(false))
        .count();
    assert_eq!(
        workspace_count, 1,
        "expected exactly 1 workspace file, found {workspace_count}"
    );

    let projects_dir = daruda_store::project::projects_dir_in(&data_dir);
    let project_count = std::fs::read_dir(&projects_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let stem = e
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            daruda_store::project::is_uuid_filename_stem(&stem)
        })
        .count();
    assert_eq!(
        project_count, 2,
        "expected 2 project files, found {project_count}"
    );

    // recent-workspaces.json is written and contains a single entry
    // for this workspace's UUID.
    let recent = daruda_store::project::load_recent_in(&data_dir);
    assert_eq!(recent.len(), 1, "exactly one recent entry");
    let ws_uuid = ws.read_with(cx, |w, _| w.uuid);
    assert_eq!(recent[0].workspace_uuid, ws_uuid);

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

#[gpui::test]
fn two_workspaces_sharing_a_project_do_not_clobber_each_other(cx: &mut TestAppContext) {
    // Smaller variant of the io.whatap scenario:
    // - W1: project A + project B, persist
    // - W2: project A alone, persist (different workspace_uuid)
    // After both persist:
    //   - 2 workspace files exist (W1.uuid.json, W2.uuid.json)
    //   - 2 project files exist (one per distinct UUID: A_w1, A_w2, B_w1
    //     — note: each workspace mints a separate UUID for its own
    //     runtime Project because `add_project` goes through
    //     `Project::bootstrap` which calls `ProjectUuid::new()`. The
    //     key invariant tested here is that W1's workspace file is
    //     still intact after W2's write.)
    //   - Loading W1 from disk yields a workspace whose project_ids
    //     still references its original two projects; W2's write did
    //     not clobber W1's workspace file.
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().to_path_buf();

    let root_a = std::env::temp_dir().join("daruda_regression_ns_share_a");
    let root_b = std::env::temp_dir().join("daruda_regression_ns_share_b");
    let _ = std::fs::create_dir_all(&root_a);
    let _ = std::fs::create_dir_all(&root_b);

    let config = daruda_config::Config::default();

    // ---- W1: A + B ----
    let project_a1 = daruda_store::project::Project::from_path(&root_a);
    let w1_handle = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project_a1),
            data_dir.clone(),
            window,
            cx,
        )
    });
    let w1 = w1_handle.root(cx).unwrap();
    cx.update_window(w1_handle.into(), |_, window, cx| {
        w1.update(cx, |ws, cx| {
            ws.add_project(root_b.clone(), window, cx);
        })
    })
    .unwrap();
    w1.read_with(cx, |w, cx| w.persist_state(cx));
    let (w1_uuid, w1_project_ids) = w1.read_with(cx, |w, _| {
        let ids: Vec<_> = w.projects.iter().map(|p| p.uuid).collect();
        (w.uuid, ids)
    });
    assert_eq!(w1_project_ids.len(), 2, "W1 holds two projects");

    // ---- W2: A alone ----
    // Use a fresh `from_path` so W2 mints its own ProjectUuid for A.
    let project_a2 = daruda_store::project::Project::from_path(&root_a);
    let w2_handle = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project_a2),
            data_dir.clone(),
            window,
            cx,
        )
    });
    let w2 = w2_handle.root(cx).unwrap();
    w2.read_with(cx, |w, cx| w.persist_state(cx));
    let w2_uuid = w2.read_with(cx, |w, _| w.uuid);

    assert_ne!(w1_uuid, w2_uuid, "W1 and W2 are distinct workspaces");

    // Invariant 1: W1's workspace file still on disk with W1's UUID.
    let w1_loaded = daruda_store::project::load_workspace_state_in(&data_dir, w1_uuid)
        .expect("W1 workspace file must survive W2's persist");
    assert_eq!(w1_loaded.uuid, w1_uuid);
    assert_eq!(
        w1_loaded.project_ids.len(),
        2,
        "W1's project list must still reference its original two projects"
    );
    for pid in &w1_project_ids {
        assert!(
            w1_loaded.project_ids.contains(pid),
            "W1's reload must still list project uuid {pid:?}"
        );
    }

    // Invariant 2: W2's workspace file on disk references one project.
    let w2_loaded = daruda_store::project::load_workspace_state_in(&data_dir, w2_uuid)
        .expect("W2 workspace file present");
    assert_eq!(w2_loaded.uuid, w2_uuid);
    assert_eq!(
        w2_loaded.project_ids.len(),
        1,
        "W2 references exactly one project"
    );

    // Invariant 3: recent-workspaces.json contains both UUIDs (most
    // recent first — W2 wrote last).
    let recent = daruda_store::project::load_recent_in(&data_dir);
    assert!(recent.len() >= 2, "recent has both entries");
    assert_eq!(recent[0].workspace_uuid, w2_uuid);
    assert!(recent.iter().any(|e| e.workspace_uuid == w1_uuid));

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}
