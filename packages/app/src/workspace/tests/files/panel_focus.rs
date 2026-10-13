use super::*;

/// Enter in the Files panel opens the file, and `open_pane_file_view` focuses
/// the pane it opened. Without handing focus back the panel loses
/// `key_context("FilesPanel")`, so the next arrow key never reaches
/// `FilesSelectNext` and keyboard browsing stops after one file. The row-click
/// path already restores it; this is the same affordance by keyboard.
#[gpui::test]
async fn enter_opens_the_file_and_keeps_the_files_panel_focused(cx: &mut TestAppContext) {
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    let a_id = child_id_by_name(&ws, cx, "a.txt");
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.file_tree.files_panel_focus.clone().focus(window, cx);
            ws.file_tree.files_selection = Some(a_id);
            ws.activate_files_selection(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        assert!(ws.focused_file_view().is_some(), "Enter must open the file");
    });
    let focused = cx
        .update_window(wh.into(), |_, window, cx| {
            ws.read(cx).file_tree.files_panel_focus.is_focused(window)
        })
        .unwrap();
    assert!(
        focused,
        "Enter handed the panel's focus to the file viewer, so the next arrow \
         key no longer reaches key_context(\"FilesPanel\")"
    );
}

/// A single click previews: it opens the file and the panel keeps keyboard
/// focus, so the arrow keys keep working. The row handler already focused the
/// panel *before* opening, which `open_pane_file_view` then undid — ordering
/// is exactly what made the intent silently ineffective, so this pins the
/// outcome rather than the sequence.
#[gpui::test]
async fn a_files_row_click_previews_and_leaves_the_panel_focused(cx: &mut TestAppContext) {
    let (wh, ws, temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    let a_id = child_id_by_name(&ws, cx, "a.txt");
    let abs = temp.path().join("a.txt");
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.on_files_row_click(id, a_id, abs.clone(), EntryKind::File, 1, false, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        let fv = ws.focused_file_view().expect("a click opens the file");
        assert_eq!(fv.path, abs);
        assert_eq!(
            ws.file_tree.files_selection,
            Some(a_id),
            "the clicked row becomes the keyboard cursor"
        );
    });
    let focused = cx
        .update_window(wh.into(), |_, window, cx| {
            ws.read(cx).file_tree.files_panel_focus.is_focused(window)
        })
        .unwrap();
    assert!(
        focused,
        "the click handed focus to the file viewer, so the arrow keys that \
         follow it never reach key_context(\"FilesPanel\")"
    );
}

/// Whatever a click means — open externally, expand a directory, preview —
/// it happened *in* the panel, so the panel ends up holding focus. Only the
/// preview branch opens a pane, so the other two would silently depend on
/// where focus already was.
#[gpui::test]
async fn every_files_row_click_leaves_the_panel_focused(cx: &mut TestAppContext) {
    let (wh, ws, temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    let a_id = child_id_by_name(&ws, cx, "a.txt");
    let sub_id = child_id_by_name(&ws, cx, "sub");
    let cases = [
        (
            a_id,
            temp.path().join("a.txt"),
            EntryKind::File,
            2usize,
            "double click",
        ),
        (
            sub_id,
            temp.path().join("sub"),
            EntryKind::Dir,
            1,
            "directory expand",
        ),
    ];

    for (entry_id, abs, kind, clicks, label) in cases {
        // Park focus outside the panel so a pass cannot come from focus
        // simply never having moved.
        cx.update_window(wh.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| {
                ws.git.panel_focus.clone().focus(window, cx);
                ws.on_files_row_click(id, entry_id, abs, kind, clicks, false, window, cx);
                let requests = ws.take_external_open_requests(cx);
                if kind.is_dir() {
                    assert!(
                        requests.is_empty(),
                        "directory clicks must not launch an app"
                    );
                } else {
                    assert_eq!(
                        requests.len(),
                        1,
                        "double click must request one external open"
                    );
                    assert_eq!(requests[0].path, temp.path().join("a.txt"));
                    assert!(
                        requests[0].preset.is_none(),
                        "the default editor must use the OS handler"
                    );
                }
            });
        })
        .unwrap();
        cx.run_until_parked();

        let focused = cx
            .update_window(wh.into(), |_, window, cx| {
                ws.read(cx).file_tree.files_panel_focus.is_focused(window)
            })
            .unwrap();
        assert!(focused, "{label} left focus outside the Files panel");
    }
}

/// Every external-file entry point uses the same test boundary, including
/// configured editors. Cleanup must be safe before background work settles.
#[gpui::test]
async fn external_file_requests_survive_fixture_cleanup_without_launching_apps(
    cx: &mut TestAppContext,
) {
    let (_wh, ws, temp) = build_workspace_with_temp_project(cx);
    let path = temp.path().join("a.txt");
    ws.update(cx, |ws, cx| {
        let lane = ws.active_ref();
        ws.mirrors.preferred_editor = "vscode".into();
        ws.open_file_externally(lane, "a.txt".into(), cx);
        ws.open_lane_file_with_system_default(lane, "a.txt".into(), cx);
        ws.open_path_with_system_default(path.clone(), cx);
    });
    temp.close().unwrap();
    cx.run_until_parked();
    ws.update(cx, |ws, cx| {
        let requests = ws.take_external_open_requests(cx);
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|request| request.path == path));
        assert_eq!(requests[0].preset.unwrap().name, "vscode");
        assert!(requests[1].preset.is_none());
        assert!(requests[2].preset.is_none());
        assert!(ws.take_external_open_requests(cx).is_empty());
    });
}

/// The Files panel needs the same keyboard door the Git panel does, and for
/// the same reason: nothing else focuses it, so its seven bindings are only
/// reachable after a click.
#[gpui::test]
async fn toggling_files_focus_switches_the_view_opens_the_dock_and_round_trips(
    cx: &mut TestAppContext,
) {
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.set_left_dock_view(daruda_store::project::LeftDockView::Lanes, cx);
            ws.docks.left.update(cx, |d, cx| {
                d.is_open = false;
                cx.notify();
            });
            ws.add_tab(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_files_focus(window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let state = cx
        .update_window(wh.into(), |_, window, cx| {
            let r = ws.read(cx);
            (
                r.docks.left.read(cx).is_open,
                r.docks.left_view,
                r.file_tree.files_panel_focus.is_focused(window),
            )
        })
        .unwrap();
    assert!(state.0, "the dock must open");
    assert_eq!(state.1, daruda_store::project::LeftDockView::Files);
    assert!(state.2, "the panel must take keyboard focus");

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_files_focus(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let still = cx
        .update_window(wh.into(), |_, window, cx| {
            ws.read(cx).file_tree.files_panel_focus.is_focused(window)
        })
        .unwrap();
    assert!(!still, "a second press must return focus to the pane");
}

/// Same two-stage Enter in the Files panel: the first opens and keeps the
/// panel focused, the second steps into the viewer.
#[gpui::test]
async fn a_second_enter_on_the_open_files_row_steps_into_the_viewer(cx: &mut TestAppContext) {
    let (wh, ws, _temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    let a_id = child_id_by_name(&ws, cx, "a.txt");
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.file_tree.files_panel_focus.clone().focus(window, cx);
            ws.file_tree.files_selection = Some(a_id);
            ws.activate_files_selection(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    let after_first = cx
        .update_window(wh.into(), |_, window, cx| {
            ws.read(cx).file_tree.files_panel_focus.is_focused(window)
        })
        .unwrap();
    assert!(after_first, "the first Enter stays in the panel");

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_files_selection(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let after_second = cx
        .update_window(wh.into(), |_, window, cx| {
            ws.read(cx).file_tree.files_panel_focus.is_focused(window)
        })
        .unwrap();
    assert!(
        !after_second,
        "a second Enter on the row already open must step into the viewer"
    );
}

/// The two left-dock panels answer the same keys, so they must answer them
/// the same way: a rested cursor previews in Files exactly as it does in Git
/// Changes, on the same debounce, without entering the pane.
#[gpui::test]
async fn files_arrow_navigation_previews_the_cursor_file(cx: &mut TestAppContext) {
    let (wh, ws, temp) = build_workspace_with_temp_project(cx);
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    // Park the cursor one row above a.txt, wherever the sort puts it, so the
    // arrow under test lands on a file rather than on whichever directory
    // happens to sort first.
    let before_a = ws.update(cx, |ws, _| {
        let visible = ws.cached_or_rebuild_visible(id);
        let pos = visible
            .iter()
            .position(|e| e.name == "a.txt")
            .expect("fixture has a.txt");
        (pos > 0).then(|| visible[pos - 1].entry_id)
    });
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.file_tree.files_panel_focus.clone().focus(window, cx);
            ws.file_tree.files_selection = before_a;
            ws.move_files_selection(1, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(
            ws.focused_file_view().is_none(),
            "the preview must wait out the debounce, not open on the keystroke"
        );
    });

    cx.executor()
        .advance_clock(std::time::Duration::from_millis(400));
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        let fv = ws
            .focused_file_view()
            .expect("a settled cursor must preview its file");
        assert_eq!(fv.path, temp.path().join("a.txt"));
    });
    let focused = cx
        .update_window(wh.into(), |_, window, cx| {
            ws.read(cx).file_tree.files_panel_focus.is_focused(window)
        })
        .unwrap();
    assert!(focused, "the preview must not take the panel's focus");
    assert!(
        !ws.read_with(cx, |ws, _| ws.input_dock.visible),
        "and must not surface the input of a pane nobody entered"
    );

    // A directory row has nothing to show, so it must leave the previous
    // preview alone rather than blanking or replacing it. Step *onto* it from
    // the row above — `move_files_selection` only arms when the selection
    // actually changes, so seeding the selection and moving by zero would
    // never reach the code under test.
    let sub_id = child_id_by_name(&ws, cx, "sub");
    let before_sub = ws.update(cx, |ws, _| {
        let visible = ws.cached_or_rebuild_visible(id);
        let pos = visible
            .iter()
            .position(|e| e.entry_id == sub_id)
            .expect("fixture has a `sub` directory");
        (pos > 0).then(|| visible[pos - 1].entry_id)
    });
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.file_tree.files_panel_focus.clone().focus(window, cx);
            ws.file_tree.files_selection = before_sub;
            ws.move_files_selection(1, window, cx);
        });
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.file_tree.files_selection,
            Some(sub_id),
            "the step must actually land on the directory row"
        );
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(400));
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.focused_file_view().map(|fv| fv.path.clone()),
            Some(temp.path().join("a.txt")),
            "resting on a directory must not disturb what is on screen"
        );
    });
}

/// Seed `text` into the focused file pane's editor, as typing would.
fn type_into_focused_file_pane(
    wh: gpui::WindowHandle<Workspace>,
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
    text: &str,
) {
    let text = text.to_string();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let pane_id = ws.active_runtime().focused_pane_id;
            let state = ws
                .active_runtime()
                .panes
                .iter()
                .find(|p| p.id == pane_id)
                .and_then(|p| p.file_content())
                .map(|fc| fc.editor_state.clone())
                .expect("focused pane is a file viewer");
            state.update(cx, |s, cx| s.set_value(text.as_str(), window, cx));
        });
    })
    .unwrap();
    cx.run_until_parked();
}

fn preview_file(
    wh: gpui::WindowHandle<Workspace>,
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
    id: daruda_store::project::LaneRef,
    path: std::path::PathBuf,
) {
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_files_entry(
                id,
                path,
                crate::workspace::main_area::tab_ops::OpenIntent::Preview,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
}

/// Unsaved edits must not be thrown away by browsing. Closing a tab already
/// asks the user what to do with them; overwriting the same tab while
/// skimming asked nothing and simply dropped them. Typing into the scratch
/// tab takes it out of the scratch slot, so the next preview opens beside it.
#[gpui::test]
async fn skimming_past_an_edited_scratch_tab_does_not_discard_the_edits(cx: &mut TestAppContext) {
    let (wh, ws, temp) = build_workspace_with_temp_project(cx);
    // The test constructor skips WindowRegistry registration, which the
    // load-completion handler needs to seed the editor — without it a loaded
    // file reads as dirty and this test would pass for the wrong reason.
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    preview_file(wh, &ws, cx, id, temp.path().join("a.txt"));
    type_into_focused_file_pane(wh, &ws, cx, "EDITED, NOT SAVED");
    ws.read_with(cx, |ws, cx| {
        let pane_id = ws.active_runtime().focused_pane_id;
        assert!(
            ws.active_runtime()
                .panes
                .iter()
                .find(|p| p.id == pane_id)
                .expect("pane")
                .is_dirty(cx),
            "the fixture must actually have unsaved edits"
        );
    });

    preview_file(wh, &ws, cx, id, temp.path().join("b.txt"));

    let panes: Vec<_> = ws.read_with(cx, |ws, _| {
        ws.active_runtime()
            .panes
            .iter()
            .filter_map(|p| p.file_view())
            .map(|fv| fv.path.clone())
            .collect()
    });
    assert!(
        panes.contains(&temp.path().join("a.txt")),
        "the edited file was replaced by the next preview, taking the unsaved \
         edits with it; open now: {panes:?}"
    );
    assert!(panes.contains(&temp.path().join("b.txt")));
}

/// The tab strip renders the scratch tab differently so the user can see which
/// one the next arrow key will take over. Both that italic and the protection
/// above read one answer, so they can never disagree: typing removes the tab
/// from the scratch slot, and the italic goes with it.
#[gpui::test]
async fn the_scratch_tab_is_identifiable_until_it_is_edited(cx: &mut TestAppContext) {
    let (wh, ws, temp) = build_workspace_with_temp_project(cx);
    // The test constructor skips WindowRegistry registration, which the
    // load-completion handler needs to seed the editor — without it a loaded
    // file reads as dirty and this test would pass for the wrong reason.
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();

    preview_file(wh, &ws, cx, id, temp.path().join("a.txt"));
    ws.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.preview_tab_index(cx),
            Some(ws.active_runtime().active_tab_index),
            "a previewed file's tab must be marked as the scratch one"
        );
    });

    type_into_focused_file_pane(wh, &ws, cx, "EDITED, NOT SAVED");
    ws.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.preview_tab_index(cx),
            None,
            "typing must clear the scratch marking, so the italic drops the \
             moment the tab stops being replaceable"
        );
    });
}
