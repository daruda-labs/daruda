//! The Git Changes list must cost the viewport, not the change set.
//!
//! It used to build one element per changed file on every dock render, which
//! is linear in the change set: measured 6.8 ms at 200 changed files and
//! 32 ms at 1000, on every pane focus change. `uniform_list` asks only for the
//! range it will paint, so the row count a render builds must stay flat as the
//! change set grows — that flatness is what this pins, rather than a wall-clock
//! budget that would be flaky and would not say why it regressed.

use gpui::{AppContext as _, TestAppContext};

use super::{Workspace, fresh_test_data_dir};

fn changed_files(n: usize) -> Vec<crate::lane::git::GitFileEntry> {
    (0..n)
        .map(|i| crate::lane::git::GitFileEntry {
            x: 'M',
            y: ' ',
            path: std::path::PathBuf::from(format!(
                "crates/app/src/module_{:03}/file_{i}.rs",
                i / 20
            )),
            original_path: None,
        })
        .collect()
}

/// Open a workspace whose left dock shows Git Changes for a git lane, with
/// `files` already in the status cache. Returns the window and workspace.
fn dock_showing_changes(
    cx: &mut TestAppContext,
    files: Vec<crate::lane::git::GitFileEntry>,
) -> (gpui::WindowHandle<Workspace>, gpui::Entity<Workspace>) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::write(root.join("a.txt"), b"hello").unwrap();
    crate::test_support::init_gpui_component(cx);
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(&root);
    let w = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = w.root(cx).unwrap();
    cx.run_until_parked();
    let repo = ws.read_with(cx, |ws, _| ws.active_lanes()[0].path.clone());
    let active = cx
        .update_window(w.into(), |_, _window, cx| {
            ws.update(cx, |ws, cx| {
                if let Some(p) = ws.active_project_mut() {
                    p.lanes[0].kind = daruda_store::project::LaneKind::Git {
                        branch: Some("main".into()),
                        repo_root: repo.clone(),
                        worktree_root: repo.clone(),
                    };
                }
                ws.left_dock.update(cx, |d, cx| {
                    d.is_open = true;
                    cx.notify();
                });
                ws.set_left_dock_view(daruda_store::project::LeftDockView::GitChanges, cx);
                ws.active_ref()
            })
        })
        .unwrap();
    ws.update(cx, |ws, _| {
        ws.git_status_cache.insert(
            active,
            crate::lane::git::GitStatusData {
                branch: Some("main".into()),
                unstaged: files,
                ..Default::default()
            },
        );
    });
    cx.run_until_parked();
    cx.update_window(w.into(), |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();
    (w, ws)
}

/// A row must span the dock, not shrink to its contents — the staging
/// checkbox is pushed to the right edge by a `flex_1` filename column, which
/// has no slack in a row that only claims its content width. Virtualizing the
/// list dropped the cross-axis stretch the old flex container supplied, and
/// the checkbox slid back against the filename with every test still green.
#[gpui::test]
async fn a_changed_file_row_spans_the_dock_so_its_checkbox_stays_right(cx: &mut TestAppContext) {
    // Short names: a content-width row would be obviously narrow, while long
    // paths could fill the dock on their own and hide the collapse.
    let files = vec![crate::lane::git::GitFileEntry {
        x: 'M',
        y: ' ',
        path: std::path::PathBuf::from("a.rs"),
        original_path: None,
    }];
    let (w, ws) = dock_showing_changes(cx, files);

    let dock_width = ws.read_with(cx, |ws, cx| ws.left_dock.read(cx).size);
    let mut vcx = gpui::VisualTestContext::from_window(w.into(), cx);
    let row = vcx
        .debug_bounds("git-changes-row")
        .expect("a changed-file row painted");

    assert!(
        f32::from(row.size.width) >= dock_width * 0.8,
        "a row for `a.rs` painted {:?} wide inside a {dock_width}px dock — it \
         is sizing to its contents, so the staging checkbox is no longer \
         pinned to the right edge",
        row.size.width
    );
}

/// Rows built by one render of the dock with `n` changed files on screen.
fn rows_built_for(cx: &mut TestAppContext, n: usize) -> usize {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::write(root.join("a.txt"), b"hello").unwrap();
    crate::test_support::init_gpui_component(cx);
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(&root);
    let w = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = w.root(cx).unwrap();
    cx.run_until_parked();

    let repo = ws.read_with(cx, |ws, _| ws.active_lanes()[0].path.clone());
    let active = cx
        .update_window(w.into(), |_, _window, cx| {
            ws.update(cx, |ws, cx| {
                if let Some(p) = ws.active_project_mut() {
                    p.lanes[0].kind = daruda_store::project::LaneKind::Git {
                        branch: Some("main".into()),
                        repo_root: repo.clone(),
                        worktree_root: repo.clone(),
                    };
                }
                ws.left_dock.update(cx, |d, cx| {
                    d.is_open = true;
                    cx.notify();
                });
                ws.set_left_dock_view(daruda_store::project::LeftDockView::GitChanges, cx);
                ws.active_ref()
            })
        })
        .unwrap();
    ws.update(cx, |ws, _| {
        ws.git_status_cache.insert(
            active,
            crate::lane::git::GitStatusData {
                branch: Some("main".into()),
                unstaged: changed_files(n),
                ..Default::default()
            },
        );
    });
    cx.run_until_parked();

    // Settle first, then count a single clean render.
    cx.update_window(w.into(), |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();
    crate::workspace::left_dock::git_changes::ROWS_BUILT.with(|c| c.set(0));
    cx.update_window(w.into(), |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();
    crate::workspace::left_dock::git_changes::ROWS_BUILT.with(|c| c.get())
}

#[gpui::test]
async fn the_changed_file_list_builds_the_viewport_not_the_change_set(cx: &mut TestAppContext) {
    let small = rows_built_for(cx, 40);
    let large = rows_built_for(cx, 2000);

    assert!(
        small > 0,
        "no rows were built at all — the list never laid out, so this test \
         would pass vacuously"
    );
    assert!(
        large < 400,
        "a render with 2000 changed files built {large} rows; the list is not \
         virtualized and the dock is linear in the change set again"
    );
    // The viewport did not change between the two, so neither should the work.
    assert!(
        large <= small * 2,
        "rows built grew with the change set: {small} at 40 files, {large} at \
         2000 — virtualization is leaking"
    );
}
