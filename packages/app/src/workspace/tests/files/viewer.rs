use super::*;

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
    cx.run_until_parked();

    ws.read_with(cx, |ws, cx| {
        let fc = ws.focused_file_content().expect("viewer open");
        assert!(
            matches!(
                fc.view.content,
                crate::workspace::main_area::file_view_pane::PaneFileContent::LoadedRaw { .. }
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

    let lane = ws.read_with(cx, |ws, _| ws.active_ref());
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_git_file_diff(
                lane,
                std::path::PathBuf::from("f.txt"),
                DiffSource::WorkingTree,
                crate::workspace::main_area::tab_ops::OpenIntent::Preview,
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

    let lane = ws.read_with(cx, |ws, _| ws.active_ref());

    // Pane A: f.txt in Changes mode, committed so the next open does not
    // reuse its tab. Becomes the focused pane.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_git_file_diff(
                lane,
                std::path::PathBuf::from("f.txt"),
                DiffSource::WorkingTree,
                crate::workspace::main_area::tab_ops::OpenIntent::Commit,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
    let pane_a_id = ws.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);

    // Pane B: g.txt in a tab of its own, which takes focus from pane A.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_pane_file_view(
                lane,
                std::path::PathBuf::from("g.txt"),
                DiffSource::WorkingTree,
                crate::workspace::main_area::file_view_pane::FileViewMode::Raw,
                crate::workspace::main_area::tab_ops::OpenIntent::Commit,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();

    let pane_b_id = ws.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
    assert_ne!(
        pane_a_id, pane_b_id,
        "the second open must focus its own pane"
    );

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

/// A file pane lives in the lane it belongs to. Asked to open in a lane
/// that does not exist, it opens nothing — there is no runtime it could
/// live in, and none other may stand in for it.
#[gpui::test]
async fn opening_into_a_missing_lane_opens_nothing(cx: &mut TestAppContext) {
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let (active, panes_before) = ws.read_with(cx, |ws, _| {
        (ws.active_ref(), ws.active_runtime().panes.len())
    });
    let missing = daruda_store::project::LaneRef {
        project: active.project,
        lane: active.lane + 1,
    };
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_pane_file_view(
                missing,
                std::path::PathBuf::from("a.txt"),
                DiffSource::WorkingTree,
                crate::workspace::main_area::file_view_pane::FileViewMode::Raw,
                crate::workspace::main_area::tab_ops::OpenIntent::Enter,
                window,
                cx,
            );
        });
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active, active, "the lane on screen stays");
        assert_eq!(ws.active_runtime().panes.len(), panes_before);
        assert!(!ws.main_area.runtimes.contains_key(&missing));
    });
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

/// `live_status` is written once at open time and is deliberately not
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
            ws.open_files_entry(
                id,
                abs.clone(),
                crate::workspace::main_area::tab_ops::OpenIntent::Enter,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.focused_file_view()
                .expect("file viewer open")
                .live_status,
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
                .live_status,
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
                .live_status,
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
        ws.update(cx, |ws, cx| {
            ws.open_files_entry(
                id,
                abs,
                crate::workspace::main_area::tab_ops::OpenIntent::Enter,
                window,
                cx,
            )
        });
    })
    .unwrap();
    assert_eq!(
        ws.read_with(cx, |ws, _| ws.active_runtime().tabs.len()),
        after_enter,
        "clicking the row Enter already opened must reuse its tab"
    );
}

/// `live_status` is derived inside `open_pane_file_view`, not supplied by the
/// caller: the agent-chat diff header, agent-chat Markdown file links, the
/// skills panel and the task form all open a file with no git context of their
/// own, and once the toolbar's mode strip started gating the Changes segment
/// on `status().is_some()` a hardcoded `None` meant a changed file opened
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
            ws.open_pane_file_view(
                id,
                abs.clone(),
                DiffSource::WorkingTree,
                FileViewMode::Raw,
                crate::workspace::main_area::tab_ops::OpenIntent::Enter,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.focused_file_view()
                .expect("file viewer open")
                .live_status,
            Some('M'),
            "a context-free open must still resolve the file's status"
        );
    });

    // Re-opening lands in the dedupe branch. Move the cache out from under the
    // pane first, so only a re-stamp there can bring it back in line.
    ws.update(cx, |ws, _cx| {
        ws.lane_scoped_mut(id).git.worktree = Some(crate::lane::git::GitWorktreeStatus::default());
    });
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_pane_file_view(
                id,
                abs.clone(),
                DiffSource::WorkingTree,
                FileViewMode::Raw,
                crate::workspace::main_area::tab_ops::OpenIntent::Enter,
                window,
                cx,
            );
        });
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.focused_file_view()
                .expect("file viewer open")
                .live_status,
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

            fc.install_content(
                PaneFileContent::LoadedRaw { truncated: false },
                Vec::new(),
                Some(window),
                cx,
            );
            assert!(
                fc.images_for_test().get(0).is_none(),
                "install_content must release the previous table, not leave it behind"
            );
        });
    })
    .unwrap();
}

/// Markdown Source renders a virtualized row list inside padding, so a thumb
/// sized from the row count alone would stop a press at the track's bottom
/// short of the last row; it must land on the real end of the content.
#[gpui::test]
async fn file_viewer_markdown_source_track_bottom_reaches_end(cx: &mut TestAppContext) {
    use crate::workspace::main_area::file_view_pane::FileViewMode;

    let (wh, ws, temp) = build_workspace_with_temp_project(cx);
    let body: String = (0..400).map(|i| format!("line {i}\n")).collect();
    std::fs::write(temp.path().join("long.md"), body).unwrap();
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_files_entry(
                id,
                std::path::PathBuf::from("long.md"),
                crate::workspace::main_area::tab_ops::OpenIntent::Enter,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.set_file_view_mode(FileViewMode::Raw, window, cx)
        });
        window.refresh();
    })
    .unwrap();
    cx.run_until_parked();

    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let strip = vcx
        .debug_bounds("file-viewer-scrollbar")
        .expect("400 Markdown source rows overflow the viewer");
    let bottom = gpui::point(strip.center().x, strip.bottom() - gpui::px(1.));
    vcx.simulate_click(bottom, gpui::Modifiers::default());
    vcx.run_until_parked();

    let (offset, max) = ws.read_with(&vcx, |ws, _| {
        let fc = ws.focused_file_content().expect("viewer open");
        (fc.scroll_handle.offset().y, fc.scroll_handle.max_offset().y)
    });
    assert!(max > gpui::px(0.), "the source rows must overflow");
    assert!(
        (offset + max).abs() < gpui::px(1.),
        "a press at the track's end must reach the content's end: offset {offset:?}, max {max:?}"
    );
}
