use gpui::Context;

use crate::workspace::Workspace;

impl Workspace {
    /// Reload config from the live store. Only wired up in tests —
    /// production goes through the `observe_global::<SettingsStore>`
    /// subscription installed in `new_with_project`.
    #[cfg(test)]
    pub fn reload_config(&mut self, user: &daruda_config::Config, cx: &mut Context<Self>) {
        let effective = effective_config_for(self.project.as_ref(), user);
        self.apply_config(&effective, cx);
    }

    /// Apply a reloaded config to all running panes. Called by the
    /// config file watcher and the Settings window when the TOML changes.
    ///
    /// **UI theme:** Workspace does *not* swap the live `DarudaTheme`
    /// here. The config watcher (`main.rs::spawn_config_watcher`) owns
    /// that — it calls `crate::ui::theme::apply_ui_theme` once per
    /// reload, app-wide, so a single config change repaints every
    /// open window. Keeping the swap out of this method means
    /// Workspace tests that build a sub-tree directly (without the
    /// full `gpui_component::init` chain) don't accidentally trigger
    /// a paint that reaches into uninitialised theme Globals.
    pub fn apply_config(&mut self, config: &daruda_config::Config, cx: &mut Context<Self>) {
        let colors = config.effective_colors();
        let pal = colors.to_ansi_palette();

        // Update terminal config for future panes.
        let fg = ghostty_vt::Rgb {
            r: colors.foreground.r,
            g: colors.foreground.g,
            b: colors.foreground.b,
        };
        let bg = ghostty_vt::Rgb {
            r: colors.background.r,
            g: colors.background.g,
            b: colors.background.b,
        };
        self.terminal_config.default_fg = fg;
        self.terminal_config.default_bg = bg;
        self.terminal_config.palette = Some(pal);
        self.terminal_config.font_size = config.font.size;
        self.terminal_config.vertical_spacing = config.font.vertical_spacing;
        self.terminal_config.horizontal_spacing = config.font.horizontal_spacing;
        self.terminal_config.clamp_font_settings();
        self.terminal_config.max_scrollback = config.scrollback.max_rows;
        self.terminal_config.background_alpha = config.window.opacity;
        self.terminal_config.osc1337_max_bytes = config.clipboard.streaming_max_bytes;
        self.font_family = config.font.family.clone();
        self.close_pane_on_exit = config.shell.close_pane_on_exit;
        self.shell_program = config.shell.program.clone();
        self.syntax_theme = config.file_viewer.syntax_theme.clone();
        self.file_viewer_preview_tab = config.file_viewer.preview_tab;
        self.notifications = config.notifications.clone();
        self.clipboard = config.clipboard.clone();
        self.claude.usage_pricing = usage_pricing_from_config(&config.usage.pricing);
        self.claude.usage_poll = config.usage.poll.clone();

        // Patch all existing pane views: font + colors + opacity.
        let font = daruda_terminal::terminal_font_with_family(&self.font_family);
        for pane in &self.main_area.panes {
            let Some(view) = pane.terminal_view() else {
                continue;
            };
            view.update(cx, |view, _cx| {
                view.set_font(font.clone());
                view.apply_font_settings(
                    config.font.size,
                    config.font.vertical_spacing,
                    config.font.horizontal_spacing,
                );
                view.apply_colors(fg, bg, &pal);
                view.set_background_alpha(config.window.opacity);
            });
        }
        // Trigger #7 — sidebar config affecting filter state changed.
        let mut filter_changed = false;
        if self.file_tree.files_show_hidden != config.sidebar.files_show_hidden {
            self.file_tree.files_show_hidden = config.sidebar.files_show_hidden;
            filter_changed = true;
        }
        if self.file_tree.files_use_gitignore != config.sidebar.files_use_gitignore {
            self.file_tree.files_use_gitignore = config.sidebar.files_use_gitignore;
            filter_changed = true;
        }
        if self.file_tree.files_icon_color_mode != config.sidebar.file_icon_color_mode {
            self.file_tree.files_icon_color_mode = config.sidebar.file_icon_color_mode.clone();
            cx.notify();
        }
        if self.panels_grid_columns != config.panels.grid_columns {
            self.panels_grid_columns = config.panels.grid_columns;
            cx.notify();
        }
        if filter_changed {
            let ids: Vec<_> = self.file_tree.file_trees.keys().copied().collect();
            for id in ids {
                self.invalidate_visible_files_cache(id);
            }
        }
        // Picks up `claude_status.enable` flips.
        let new_enabled = config.claude_status.enable;
        if new_enabled != self.claude.claude_status_enabled {
            self.claude.claude_status_enabled = new_enabled;
            self.refresh_jsonl_watcher(cx);
        }
        cx.notify();
    }
}

/// Layer the project-local override on top of the user-global config
/// and return the resolved [`daruda_config::Config`].
pub(in crate::workspace) fn effective_config_for(
    project: Option<&daruda_store::project::Project>,
    user: &daruda_config::Config,
) -> daruda_config::Config {
    let project_cfg = project
        .map(|p| daruda_config::ProjectConfig::load_for(&p.root))
        .unwrap_or_default();
    user.clone().resolve(&project_cfg)
}

/// Translate `[usage.pricing]` (TOML-facing) into the data-layer
/// [`daruda_claude::usage::UsagePricing`].
pub(in crate::workspace) fn usage_pricing_from_config(
    p: &daruda_config::PricingConfig,
) -> daruda_claude::usage::UsagePricing {
    daruda_claude::usage::UsagePricing {
        input_per_mtok: p.input_per_mtok,
        output_per_mtok: p.output_per_mtok,
        cache_read_per_mtok: p.cache_read_per_mtok,
        cache_write_per_mtok: p.cache_write_per_mtok,
    }
}
