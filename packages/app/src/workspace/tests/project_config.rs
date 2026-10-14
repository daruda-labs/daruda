//! The status-bar project-config dot follows what the project layer
//! overrides, not whether its file exists.

use super::*;
use gpui::BorrowAppContext as _;

#[gpui::test]
fn the_dot_tracks_an_override_not_the_files_existence(cx: &mut TestAppContext) {
    init_gpui_component(cx);
    let root = tempfile::tempdir().unwrap();
    let data_dir = fresh_test_data_dir();
    let path = daruda_config::project_config_path_in(&data_dir, root.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // What "Open Project Config" writes: every line commented out.
    std::fs::write(&path, daruda_config::project::PROJECT_CONFIG_TEMPLATE).unwrap();

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(root.path());
    let handle = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(&config, Some(project), data_dir, window, cx)
    });
    let ws = handle.root(cx).unwrap();

    ws.update(cx, |ws, _| assert!(!ws.project_layer_overrides()));

    std::fs::write(&path, "[shell]\nprogram = \"/bin/project-shell\"\n").unwrap();
    ws.update(cx, |ws, cx| {
        ws.reload_config(&config, cx);
        assert!(ws.project_layer_overrides());
    });
}

/// The user layer's own shell, in the store's `config.toml`.
const USER_TOML: &str = "[shell]\nprogram = \"/bin/user-shell\"\n";

/// Build a window on `root` whose project layer holds `layer`, the live store
/// holding [`USER_TOML`] as its user layer.
fn workspace_with_layer(
    cx: &mut TestAppContext,
    root: &std::path::Path,
    layer: &str,
) -> gpui::Entity<Workspace> {
    init_gpui_component(cx);
    let data_dir = fresh_test_data_dir();
    let path = daruda_config::project_config_path_in(&data_dir, root);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, layer).unwrap();
    let user = cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.set_user_for_testing(daruda_config::Config::default());
            std::fs::write(store.writer_path_for_testing(), USER_TOML).unwrap();
            store.reload_user();
            store.user().clone()
        })
    });
    let project = daruda_store::project::Project::from_path(root);
    let handle = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(&user, Some(project), data_dir, window, cx)
    });
    handle.root(cx).unwrap()
}

/// A settings change through the live store — what a Settings save or a
/// config-file tick does — the path production takes into `apply_config`.
fn change_a_setting(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::TerminalFontSize(15.0))
                .expect("settings patch");
        });
    });
    cx.run_until_parked();
}

#[gpui::test]
fn a_settings_change_keeps_the_project_shell_and_its_dot(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let ws = workspace_with_layer(
        cx,
        root.path(),
        "[shell]\nprogram = \"/bin/project-shell\"\n",
    );

    change_a_setting(cx);

    ws.update(cx, |ws, _| {
        assert_eq!(
            ws.mirrors.shell_program.as_deref(),
            Some("/bin/project-shell"),
            "a settings change must not drop the project layer's shell"
        );
        assert!(ws.project_layer_overrides());
    });
}

#[gpui::test]
fn a_template_only_layer_keeps_the_user_shell_and_no_dot(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let ws = workspace_with_layer(
        cx,
        root.path(),
        daruda_config::project::PROJECT_CONFIG_TEMPLATE,
    );

    change_a_setting(cx);

    ws.update(cx, |ws, _| {
        assert_eq!(ws.mirrors.shell_program.as_deref(), Some("/bin/user-shell"));
        assert!(!ws.project_layer_overrides());
    });
}
