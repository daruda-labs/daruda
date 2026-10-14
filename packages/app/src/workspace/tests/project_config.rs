//! The status-bar project-config dot follows what the project layer
//! overrides, not whether its file exists.

use super::*;

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
