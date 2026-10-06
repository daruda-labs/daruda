use daruda_config::{Config, IconColorMode};
use gpui::{BorrowAppContext as _, TestAppContext};

use super::*;

#[gpui::test]
async fn apply_config_syncs_all_mirrors(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);

    // Snapshot the defaults so the test remains valid if defaults change.
    let baseline = ws.read_with(cx, |ws, _| ws.mirrors.clone());

    let mut new_config = Config::default();
    new_config.panels.grid_columns = baseline.panels_grid_columns.wrapping_add(1);
    new_config.shell.close_pane_on_exit = !baseline.close_pane_on_exit;
    new_config.shell.program = Some("/bin/test-shell".into());
    new_config.left_dock.files_show_hidden = !baseline.files_show_hidden;
    new_config.left_dock.files_use_gitignore = !baseline.files_use_gitignore;
    new_config.left_dock.file_icon_color_mode = match baseline.files_icon_color_mode {
        IconColorMode::Color => IconColorMode::Monochrome,
        IconColorMode::Monochrome => IconColorMode::Color,
    };
    // A value distinct from the shipped default, so the assertion proves the
    // mirror synced rather than matching the baseline by accident.
    new_config.agent.hidden_config_option_descriptions = vec!["mirror-sync-probe".to_string()];

    ws.update(cx, |ws, cx| ws.apply_config(&new_config, cx));

    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.mirrors.panels_grid_columns,
            new_config.panels.grid_columns
        );
        assert_eq!(
            ws.mirrors.close_pane_on_exit,
            new_config.shell.close_pane_on_exit
        );
        assert_eq!(ws.shell_program.as_deref(), Some("/bin/test-shell"));
        assert_eq!(
            ws.mirrors.files_show_hidden,
            new_config.left_dock.files_show_hidden
        );
        assert_eq!(
            ws.mirrors.files_use_gitignore,
            new_config.left_dock.files_use_gitignore
        );
        assert_eq!(
            ws.mirrors.files_icon_color_mode,
            new_config.left_dock.file_icon_color_mode
        );
        assert_eq!(
            ws.mirrors.hidden_config_option_descriptions,
            new_config.agent.hidden_config_option_descriptions
        );
    });

    let before = ws.read_with(cx, |ws, _| ws.mirrors.files_show_hidden);
    ws.update(cx, |ws, cx| ws.toggle_files_show_hidden(cx));
    let after = ws.read_with(cx, |ws, _| ws.mirrors.files_show_hidden);
    assert_eq!(after, !before);
}

/// An OS flip under `ui_preset = "system"`: the appearance arrives as a value
/// (the platform must not be asked again from inside the observer), the live
/// theme follows it, and the mirror that gates the file-pane reload moves.
#[gpui::test]
async fn a_system_appearance_flip_swaps_the_theme_and_the_mirror(cx: &mut TestAppContext) {
    use gpui::WindowAppearance;
    let (_wh, ws) = build_workspace(cx);
    cx.update(|cx| {
        crate::ui::theme::init_if_missing(cx);
        crate::settings_store::SettingsStore::init(cx);
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            let mut cfg = Config::default();
            cfg.theme.ui_preset = daruda_config::ui_theme_presets::SYSTEM.to_owned();
            store.set_user_for_testing(cfg);
        });
        crate::ui::theme::init_ui_theme(daruda_config::ui_theme_presets::SYSTEM, cx);
        crate::ui::theme::note_system_appearance(WindowAppearance::Light, cx);
    });

    ws.update(cx, |ws, cx| {
        ws.on_system_appearance_changed(WindowAppearance::Dark, cx)
    });
    ws.read_with(cx, |ws, cx| {
        assert!(crate::ui::theme::current(cx).is_dark());
        assert_eq!(ws.mirrors.painted_ui_preset, "daruda_dark");
    });

    ws.update(cx, |ws, cx| {
        ws.on_system_appearance_changed(WindowAppearance::Light, cx)
    });
    ws.read_with(cx, |ws, cx| {
        assert!(!crate::ui::theme::current(cx).is_dark());
        assert_eq!(ws.mirrors.painted_ui_preset, "daruda_light");
    });
}

/// An explicit preset ignores the OS: the flip is recorded, nothing repaints.
#[gpui::test]
async fn a_system_appearance_flip_leaves_an_explicit_preset_alone(cx: &mut TestAppContext) {
    use gpui::WindowAppearance;
    let (_wh, ws) = build_workspace(cx);
    cx.update(|cx| {
        crate::ui::theme::init_if_missing(cx);
        crate::settings_store::SettingsStore::init(cx);
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.set_user_for_testing(Config::default());
        });
        crate::ui::theme::init_ui_theme("daruda_dark", cx);
    });
    ws.update(cx, |ws, cx| {
        ws.on_system_appearance_changed(WindowAppearance::Light, cx)
    });
    ws.read_with(cx, |ws, cx| {
        assert!(crate::ui::theme::current(cx).is_dark(), "daruda_dark stays");
        assert_eq!(ws.mirrors.painted_ui_preset, "daruda_dark");
        assert_eq!(
            crate::ui::theme::painted_ui_preset(daruda_config::ui_theme_presets::SYSTEM, cx),
            "daruda_light",
            "the flip is still recorded for a later switch to `system`"
        );
    });
}

/// Two windows, one settings change: each diffs against what *it* last
/// applied. The shared-surface values are also written to app-wide globals,
/// and diffing against those told only the first window anything moved —
/// the rest skipped their file-pane reload and chat re-measure.
#[gpui::test]
async fn a_second_window_still_sees_a_shared_setting_change(cx: &mut TestAppContext) {
    let (_wh1, first) = build_workspace(cx);
    let (_wh2, second) = build_workspace(cx);
    let before = second.read_with(cx, |ws, _| ws.mirrors.shared_surface.clone());

    let mut changed = Config::default();
    changed.font.editor.size += 3.0;
    changed.font.agent_chat.size += 2.0;
    changed.agent.reading_width += 40.0;
    changed.window.opacity = 0.5;
    first.update(cx, |ws, cx| ws.apply_config(&changed, cx));

    second.read_with(cx, |ws, _| {
        assert!(
            ws.mirrors.shared_surface == before,
            "another window's apply must not move this window's baseline"
        );
    });
    second.update(cx, |ws, cx| ws.apply_config(&changed, cx));
    second.read_with(cx, |ws, _| {
        let now = &ws.mirrors.shared_surface;
        assert!(now.editor_font != before.editor_font);
        assert!(now.agent_chat_font != before.agent_chat_font);
        assert!(now.agent_chat_reading_width != before.agent_chat_reading_width);
        assert!(now.window_opacity != before.window_opacity);
    });
}

/// The behaviour behind the baseline test above: the second window really
/// re-bakes its file panes. The file changes on disk under an open markdown
/// pane — test builds run no file watcher — so only a reload shows the new
/// text; the old code let the first window's global write tell the second
/// window "unchanged", and its pane kept the old text.
#[gpui::test]
async fn a_second_window_reloads_its_file_panes_for_a_shared_setting(cx: &mut TestAppContext) {
    use crate::workspace::main_area::file_view_pane::{DiffSource, FileViewMode, PaneFileContent};
    use crate::workspace::main_area::tab_ops::OpenIntent;

    let temp = tempfile::tempdir().unwrap();
    let root = daruda_core::path::canonicalize(temp.path()).unwrap();
    let doc = root.join("notes.md");
    std::fs::write(&doc, "first draft\n").unwrap();
    let (_wh1, first) = build_workspace(cx);
    let (wh2, second) = build_workspace_with(
        cx,
        &Config::default(),
        Some(daruda_store::project::Project::from_path(&root)),
    );
    cx.update_window(wh2.into(), |_, window, cx| {
        second.update(cx, |ws, cx| {
            let lane = ws.active.lane;
            ws.open_pane_file_view(
                lane,
                doc.clone(),
                DiffSource::WorkingTree,
                FileViewMode::Preview,
                OpenIntent::Commit,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
    let shows = |cx: &mut TestAppContext, text: &str| {
        second.read_with(cx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .filter_map(|p| p.file_view())
                .find(|fv| fv.path == doc)
                .is_some_and(|fv| match &fv.content {
                    PaneFileContent::LoadedMarkdown { raw_rows, .. } => {
                        raw_rows.iter().any(|row| row.content.contains(text))
                    }
                    _ => false,
                })
        })
    };
    assert!(shows(cx, "first draft"), "the pane loaded the file");

    std::fs::write(&doc, "second draft\n").unwrap();
    let unchanged = Config::default();
    second.update(cx, |ws, cx| ws.apply_config(&unchanged, cx));
    cx.run_until_parked();
    assert!(
        shows(cx, "first draft"),
        "a reload with nothing moved re-bakes nothing"
    );

    let mut changed = Config::default();
    changed.font.editor.size += 3.0;
    first.update(cx, |ws, cx| ws.apply_config(&changed, cx));
    second.update(cx, |ws, cx| ws.apply_config(&changed, cx));
    cx.run_until_parked();
    assert!(
        shows(cx, "second draft"),
        "the second window re-baked its pane"
    );
}
