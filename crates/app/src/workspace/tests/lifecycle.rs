use super::*;

// ---- Render regression ----

#[gpui::test]
async fn test_workspace_renders_without_reentrant_panic(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, _cx| {
        window.refresh();
    })
    .unwrap();

    workspace.update(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(ws.active_runtime().active_tab_index, 0);
    });
}

// ---- Modal layer ----
//
// Phase 4.c+4.d removed `Workspace::open_modal`/`dismiss_modal`/
// `active_modal` and `ModalLayer`. Modal lifecycle is now owned by
// `gpui_component::Root` (via `crate::workspace::dialog_helpers`),
// which routes through `Window::open_dialog` / `close_dialog`. Those
// APIs require a real `Root`-wrapped window — covered by visual smoke
// testing rather than `TestAppContext` here.

// ---- Tab add / close / switch ----

#[gpui::test]
async fn test_add_tab(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1);
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 2);
        assert_eq!(ws.active_runtime().active_tab_index, 1);
    });
}

#[gpui::test]
async fn test_set_window_label_sets_then_clears(cx: &mut TestAppContext) {
    let (_window_handle, workspace) = build_workspace(cx);

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
}

#[gpui::test]
async fn test_close_tab_removes_tab(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            ws.add_tab(window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 3);
        assert_eq!(ws.active_runtime().active_tab_index, 2);
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_close_tab(&CloseTab, window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 2);
        assert_eq!(ws.active_runtime().active_tab_index, 1);
    });
}

#[gpui::test]
async fn test_close_non_active_tab_preserves_active(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            ws.add_tab(window, cx);
        });
    })
    .unwrap();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_tab_at(0, window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 2);
        assert_eq!(ws.active_runtime().active_tab_index, 1);
    });
}

#[gpui::test]
async fn test_next_prev_tab_cycles(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            ws.add_tab(window, cx);
        });
    })
    .unwrap();

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
}

#[gpui::test]
async fn test_activate_tab_by_index(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            ws.add_tab(window, cx);
        });
    })
    .unwrap();

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
}

#[gpui::test]
async fn test_cmd9_activates_last_tab(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            ws.add_tab(window, cx);
        });
    })
    .unwrap();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.activate_tab(0, window, cx);
        });
    })
    .unwrap();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_activate_tab_n(8, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().active_tab_index, 2)
    });
}

#[gpui::test]
async fn test_tab_ids_are_unique_and_stable(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            ws.add_tab(window, cx);
        });
    })
    .unwrap();

    let ids_before: Vec<u64> = workspace.read_with(cx, |ws, _| {
        ws.active_runtime().tabs.iter().map(|t| t.id).collect()
    });
    assert_eq!(ids_before.len(), 3);
    assert!(ids_before[0] != ids_before[1] && ids_before[1] != ids_before[2]);

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
}

// ---- Resize regression ----

#[gpui::test]
async fn test_resize_all_tabs_no_reentrant_panic(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
        });
    })
    .unwrap();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.resize_all_tabs(window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 2);
    });
}

#[gpui::test]
async fn test_observe_window_bounds_no_reentrant_panic(cx: &mut TestAppContext) {
    use gpui::size;

    let config = daruda_config::Config::default();
    let window_handle =
        cx.add_window(|window, cx| Workspace::new(&config, fresh_test_data_dir(), window, cx));
    let workspace = window_handle.root(cx).unwrap();

    cx.simulate_window_resize(window_handle.into(), size(gpui::px(800.0), gpui::px(600.0)));

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1);
    });
}

// ---- PaneSpawnError integration ----

#[test]
fn test_pane_spawn_error_display_pty_variant() {
    use crate::workspace::main_area::pane::PaneSpawnError;
    use daruda_terminal::pty::PtyError;

    let err = PaneSpawnError::Pty(PtyError::SpawnShell("not found".into()));
    let text = err.to_string();
    assert!(text.starts_with("PTY:"), "unexpected prefix: {text}");
    assert!(text.contains("not found"), "error body missing: {text}");
}

#[test]
fn test_pane_spawn_error_is_std_error() {
    use crate::workspace::main_area::pane::PaneSpawnError;
    use daruda_terminal::pty::PtyError;

    // Ensures PaneSpawnError satisfies std::error::Error so it can be
    // boxed / propagated through `?` alongside other std error types.
    let boxed: Box<dyn std::error::Error> =
        Box::new(PaneSpawnError::Pty(PtyError::OpenPty("boom".into())));
    assert!(!boxed.to_string().is_empty());
}

#[gpui::test]
fn test_report_pane_error_sets_last_error(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::PaneSpawnError;
    use daruda_terminal::pty::PtyError;

    let (window_handle, workspace) = build_workspace(cx);

    workspace.read_with(cx, |ws, _| assert!(ws.last_error.is_none()));

    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.report_pane_error(
                "new tab",
                PaneSpawnError::Pty(PtyError::SpawnShell("exec fail".into())),
                cx,
            );
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        let msg = ws.last_error.as_ref().expect("last_error should be set");
        assert!(msg.contains("new tab"), "context missing: {msg}");
        assert!(msg.contains("exec fail"), "cause missing: {msg}");
    });
}

#[gpui::test]
fn test_report_pane_error_does_not_mutate_layout(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::PaneSpawnError;

    let (window_handle, workspace) = build_workspace(cx);
    let (tabs_before, panes_before) = workspace.read_with(cx, |ws, _| {
        (
            ws.active_runtime().tabs.len(),
            ws.active_runtime().panes.len(),
        )
    });

    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.report_pane_error(
                "split",
                PaneSpawnError::Vt(daruda_terminal::VtError::CreateFailed),
                cx,
            );
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active_runtime().tabs.len(),
            tabs_before,
            "tabs should be untouched"
        );
        assert_eq!(
            ws.active_runtime().panes.len(),
            panes_before,
            "panes should be untouched"
        );
        assert!(ws.last_error.is_some());
    });
}

// ---- close_pane_on_exit config plumbing ----

#[gpui::test]
fn test_close_pane_on_exit_defaults_true(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, _| {
        assert!(ws.mirrors.close_pane_on_exit);
    });
}

#[gpui::test]
fn test_sibling_close_pattern_no_reentrant_panic(cx: &mut TestAppContext) {
    // Mirrors the stdout-poll sibling task's pattern: read the config
    // flag, then in a separate update close the pane. If GPUI ever
    // tightens reentrancy rules this test will fail fast at the
    // borrow boundary rather than deep inside a PTY task.
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
fn test_apply_config_updates_close_pane_on_exit(cx: &mut TestAppContext) {
    let (window_handle, ws) = build_workspace(cx);
    let mut cfg = daruda_config::Config::default();
    cfg.shell.close_pane_on_exit = false;

    cx.update_window(window_handle.into(), |_, _window, cx| {
        ws.update(cx, |ws, cx| ws.apply_config(&cfg, cx));
    })
    .unwrap();
    ws.read_with(cx, |ws, _| assert!(!ws.mirrors.close_pane_on_exit));

    cfg.shell.close_pane_on_exit = true;
    cx.update_window(window_handle.into(), |_, _window, cx| {
        ws.update(cx, |ws, cx| ws.apply_config(&cfg, cx));
    })
    .unwrap();
    ws.read_with(cx, |ws, _| assert!(ws.mirrors.close_pane_on_exit));
}

#[gpui::test]
fn test_apply_config_propagates_shell_program(cx: &mut TestAppContext) {
    // The user-`[shell]` `program` (and any project-layer override of
    // it via `Config::resolve`) reaches new panes through
    // `Workspace::shell_program`. This test exercises the apply path
    // end-to-end so a future refactor of `apply_config` doesn't
    // silently drop the field.
    let (window_handle, ws) = build_workspace(cx);
    let mut cfg = daruda_config::Config::default();
    cfg.shell.program = Some("/bin/test-shell".into());

    cx.update_window(window_handle.into(), |_, _window, cx| {
        ws.update(cx, |ws, cx| ws.apply_config(&cfg, cx));
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.shell_program.as_deref(), Some("/bin/test-shell"));
    });

    // Clearing the field at the user layer (with no project override)
    // resets the workspace back to the "use $SHELL/zsh default" path.
    cfg.shell.program = None;
    cx.update_window(window_handle.into(), |_, _window, cx| {
        ws.update(cx, |ws, cx| ws.apply_config(&cfg, cx));
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        assert!(ws.shell_program.is_none());
    });
}

#[gpui::test]
fn test_reload_config_resolves_project_layer_shell_override(cx: &mut TestAppContext) {
    // `reload_config` must apply the project layer on top of the
    // user config it receives. This test materialises a real project
    // config file at the path `daruda_config::project_config_path`
    // would compute for a temp repo, so the resolve goes through the
    // actual `ProjectConfig::load_for` lookup rather than a hand-
    // assembled merge.
    let temp = tempfile::tempdir().unwrap();
    let project_path = temp.path().to_path_buf();
    let cfg_path = daruda_config::project_config_path(&project_path)
        .expect("project_config_path must yield a path on this platform");
    if let Some(parent) = cfg_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(
        &cfg_path,
        "[shell]\nprogram = \"/bin/project-shell\"\nclose_pane_on_exit = false\n",
    )
    .unwrap();
    // Clean up the on-disk file at end of test so we don't pollute
    // the user's real `~/.config/daruda/projects/` tree.
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
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
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

#[gpui::test]
fn test_last_error_surfaces_in_status_bar_data(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::PaneSpawnError;
    use daruda_terminal::pty::PtyError;

    let (window_handle, workspace) = build_workspace(cx);
    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.report_pane_error(
                "spawn",
                PaneSpawnError::Pty(PtyError::OpenPty("enomem".into())),
                cx,
            );
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        // The render path copies `last_error` into StatusBarData.error
        // (see render.rs). Assert the source field is what we expect —
        // rendering itself is verified by the no-panic render tests.
        let err = ws.last_error.as_ref().expect("error set");
        assert!(err.contains("spawn"));
        assert!(err.contains("enomem"));
    });
}
