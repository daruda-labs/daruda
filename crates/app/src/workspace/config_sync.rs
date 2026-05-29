//! Single source of truth for config-derived fields cached on
//! `Workspace`. `ConfigMirrors::from_config` is the only place that
//! reads `daruda_config::Config` into mirror state — adding a new
//! mirror field here forces the sync site to be updated, so sync
//! omissions become compile errors instead of silent state divergence.

use std::time::Duration;

use daruda_config::{Config, IconColorMode};

#[derive(Clone)]
pub(in crate::workspace) struct ConfigMirrors {
    /// Mirror of `daruda_config::PanelsConfig::grid_columns`. Drives
    /// the macro-key column count in `MacroDock`.
    pub panels_grid_columns: u8,

    /// Mirror of `daruda_config::ShellConfig::close_pane_on_exit`.
    /// When true, a pane closes itself after the PTY exits.
    pub close_pane_on_exit: bool,

    /// Mirror of `daruda_config::LeftDockConfig::files_show_hidden`.
    /// Drives the dotfile filter inside the Files view's `walk_into`.
    /// Toggled at runtime by `FilesToggleHidden`; `apply_config`
    /// overwrites it on live reload.
    pub files_show_hidden: bool,

    /// Mirror of `daruda_config::LeftDockConfig::files_use_gitignore`.
    /// When true, `walk_into` consults `files_gitignore_index` per row.
    pub files_use_gitignore: bool,

    /// Mirror of `daruda_config::LeftDockConfig::file_icon_color_mode`.
    pub files_icon_color_mode: IconColorMode,

    /// PTY-output batch / repaint interval derived from
    /// `daruda_config::RenderConfig::max_fps` (`1000 / max_fps` ms).
    /// Read by the per-pane stdout poll loop to cap how often terminal
    /// output triggers a repaint; live-updates on config reload.
    pub terminal_redraw_interval: Duration,
}

impl ConfigMirrors {
    pub(in crate::workspace) fn from_config(config: &Config) -> Self {
        Self {
            panels_grid_columns: config.panels.grid_columns,
            close_pane_on_exit: config.shell.close_pane_on_exit,
            files_show_hidden: config.left_dock.files_show_hidden,
            files_use_gitignore: config.left_dock.files_use_gitignore,
            files_icon_color_mode: config.left_dock.file_icon_color_mode.clone(),
            terminal_redraw_interval: config.render.redraw_interval(),
        }
    }
}

#[cfg(test)]
mod tests {
    // Integration coverage lives in workspace/tests/config_mirror.rs —
    // apply_config_syncs_all_mirrors + toggle_files_show_hidden_flips_mirror.
}
