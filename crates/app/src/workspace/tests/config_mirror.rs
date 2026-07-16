use daruda_config::{Config, IconColorMode};
use gpui::TestAppContext;

use super::*;

#[gpui::test]
async fn apply_config_syncs_all_mirrors(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);

    // Snapshot the defaults so the test remains valid if defaults change.
    let baseline = ws.read_with(cx, |ws, _| ws.mirrors.clone());

    let mut new_config = Config::default();
    new_config.panels.grid_columns = baseline.panels_grid_columns.wrapping_add(1);
    new_config.shell.close_pane_on_exit = !baseline.close_pane_on_exit;
    new_config.left_dock.files_show_hidden = !baseline.files_show_hidden;
    new_config.left_dock.files_use_gitignore = !baseline.files_use_gitignore;
    new_config.left_dock.file_icon_color_mode = match baseline.files_icon_color_mode {
        IconColorMode::Color => IconColorMode::Monochrome,
        IconColorMode::Monochrome => IconColorMode::Color,
    };

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
    });
}

#[gpui::test]
async fn toggle_files_show_hidden_flips_mirror(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    let before = ws.read_with(cx, |ws, _| ws.mirrors.files_show_hidden);
    ws.update(cx, |ws, cx| ws.toggle_files_show_hidden(cx));
    let after = ws.read_with(cx, |ws, _| ws.mirrors.files_show_hidden);
    assert_eq!(after, !before);
}
