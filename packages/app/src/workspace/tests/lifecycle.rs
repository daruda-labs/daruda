use super::*;

// ---- Modal layer ----
//
// Modal lifecycle is owned by `gpui_component::Root`, which requires a real
// `Root`-wrapped window — covered by visual smoke testing, not `TestAppContext`.

// ---- Tab add / close / switch ----

#[gpui::test]
async fn tab_add_close_switching_and_ids_share_one_three_tab_fixture(cx: &mut TestAppContext) {
    use gpui::size;

    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, _cx| {
        window.refresh();
    })
    .unwrap();
    cx.simulate_window_resize(window_handle.into(), size(gpui::px(800.0), gpui::px(600.0)));

    workspace.update(cx, |ws, cx| {
        ws.set_window_label(Some("daruda — review".into()), cx);
    });
    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.window_user_label.as_ref().map(|s| s.as_ref()),
            Some("daruda — review"),
            "window_user_label was not stored"
        );
    });

    workspace.update(cx, |ws, cx| {
        ws.set_window_label(None, cx);
    });
    workspace.read_with(cx, |ws, _| {
        assert!(
            ws.window_user_label.is_none(),
            "window_user_label was not cleared by None"
        );
    });

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(ws.active_runtime().active_tab_index, 0);
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            ws.add_tab(window, cx);
            ws.resize_all_tabs(window, cx);
        });
    })
    .unwrap();

    let ids_before: Vec<u64> = workspace.read_with(cx, |ws, _| {
        ws.active_runtime().tabs.iter().map(|t| t.id).collect()
    });
    assert_eq!(ids_before.len(), 3);
    assert_ne!(ids_before[0], ids_before[1]);
    assert_ne!(ids_before[0], ids_before[2]);
    assert_ne!(ids_before[1], ids_before[2]);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_next_tab(&NextTab, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().active_tab_index, 0)
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_prev_tab(&PrevTab, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().active_tab_index, 2)
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_activate_tab_n(0, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().active_tab_index, 0)
    });

    // Out-of-bounds is a no-op
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_activate_tab_n(99, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().active_tab_index, 0)
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_activate_tab_n(8, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().active_tab_index, 2)
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_tab_at(1, window, cx);
        });
    })
    .unwrap();

    let ids_after: Vec<u64> = workspace.read_with(cx, |ws, _| {
        ws.active_runtime().tabs.iter().map(|t| t.id).collect()
    });
    assert_eq!(ids_after, vec![ids_before[0], ids_before[2]]);
    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active_runtime().active_tab_index,
            1,
            "closing a non-active tab preserves the active tab"
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_close_tab(&CloseTab, window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(ws.active_runtime().active_tab_index, 0);
        assert_eq!(ws.active_runtime().tabs[0].id, ids_before[0]);
    });
}

// ---- PaneSpawnError integration ----

#[test]
fn pane_spawn_error_display_and_std_error() {
    use crate::workspace::main_area::pane::PaneSpawnError;
    use daruda_terminal::pty::PtyError;

    let err = PaneSpawnError::Pty(PtyError::SpawnShell("not found".into()));
    let text = err.to_string();
    assert!(!text.is_empty(), "unexpected empty message");
    assert!(text.contains("not found"), "error body missing: {text}");

    let boxed: Box<dyn std::error::Error> =
        Box::new(PaneSpawnError::Pty(PtyError::OpenPty("boom".into())));
    assert!(!boxed.to_string().is_empty());
}

// ---- close_pane_on_exit config plumbing ----

#[gpui::test]
fn test_sibling_close_pattern_no_reentrant_panic(cx: &mut TestAppContext) {
    // Mirrors the stdout-poll sibling task's pattern: read the config flag,
    // then close the pane in a separate update. Guards against a reentrant
    // borrow conflict at that boundary.
    let (window_handle, ws) = build_workspace(cx);
    cx.update_window(window_handle.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.add_tab(window, cx));
    })
    .unwrap();
    let pane_id = ws.read_with(cx, |ws, _| ws.active_runtime().panes[0].id);

    cx.update_window(window_handle.into(), |_, window, cx| {
        let should_close = ws.read(cx).mirrors.close_pane_on_exit;
        if should_close {
            ws.update(cx, |ws, cx| ws.close_pane_by_id(pane_id, window, cx));
        }
    })
    .unwrap();

    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1, "one tab should remain");
        assert_eq!(ws.active_runtime().panes.len(), 1, "one pane should remain");
    });
}

#[gpui::test]
fn test_reload_config_resolves_project_layer_shell_override(cx: &mut TestAppContext) {
    // `reload_config` must apply the project layer on top of the user config.
    // Materialises a real project config file at the `project_config_path`
    // location so the resolve goes through the actual `ProjectConfig::load_for`
    // lookup rather than a hand-assembled merge.
    let temp = tempfile::tempdir().unwrap();
    let project_path = temp.path().to_path_buf();
    let data_dir = fresh_test_data_dir();
    let cfg_path = daruda_config::project_config_path_in(&data_dir, &project_path);
    if let Some(parent) = cfg_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(
        &cfg_path,
        "[shell]\nprogram = \"/bin/project-shell\"\nclose_pane_on_exit = false\n",
    )
    .unwrap();
    // Clean up the on-disk file so the test doesn't pollute the real
    // `~/.config/daruda/projects/` tree.
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            if let Some(parent) = self.0.parent() {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }
    let _cleanup = Cleanup(cfg_path);

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(&project_path);
    let window_handle = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(&config, Some(project), data_dir, window, cx)
    });
    let ws = window_handle.root(cx).unwrap();

    // `new_with_project` already resolves the project layer once.
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.shell_program.as_deref(),
            Some("/bin/project-shell"),
            "project-layer shell.program must override user default at construction",
        );
        assert!(
            !ws.mirrors.close_pane_on_exit,
            "project-layer shell section replaces wholesale, including close_pane_on_exit",
        );
    });

    // A live reload with the same user config should re-pick up the
    // (unchanged) project file rather than falling back to user.
    cx.update_window(window_handle.into(), |_, _window, cx| {
        ws.update(cx, |ws, cx| ws.reload_config(&config, cx));
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.shell_program.as_deref(), Some("/bin/project-shell"));
    });
}
