use super::*;

pub(super) struct WorkspaceSelects {
    pub(super) editor_select: Entity<SelectState>,
    pub(super) left_dock_size_select: Entity<SelectState>,
    pub(super) status_icon_theme_select: Entity<SelectState>,
    pub(super) file_icon_color_select: Entity<SelectState>,
}

impl WorkspaceSelects {
    pub(super) fn new(
        config: &Config,
        themes: &[(String, String)],
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> Self {
        // External editor select — "" (empty, the config default) means the
        // OS default handler; every other value is a `daruda_config::editor`
        // preset name.
        let preferred_editor = SharedString::from(config.editor.preferred.clone());
        let editor_select = cx.new(|cx| {
            let mut opts = vec![select::SelectOption::new(
                "",
                s::settings::editor_system_default(),
            )];
            opts.extend(
                daruda_config::EXTERNAL_EDITOR_PRESETS
                    .iter()
                    .map(|p| select::SelectOption::new(p.name, p.display_name)),
            );
            select::state_with_options(opts, Some(&preferred_editor), window, cx)
        });
        let left_dock_size_select = cx.new(|cx| {
            let opts = daruda_config::UiSize::ALL
                .into_iter()
                .map(|size| {
                    let label = match size {
                        daruda_config::UiSize::Small => s::settings::ui_size_small(),
                        daruda_config::UiSize::Normal => s::settings::ui_size_normal(),
                        daruda_config::UiSize::Large => s::settings::ui_size_large(),
                    };
                    SelectOption::new(size.as_str(), label)
                })
                .collect();
            let current = SharedString::new_static(config.left_dock.size.as_str());
            select::state_with_options(opts, Some(&current), window, cx)
        });
        let status_icon_theme_select = cx.new(|cx| {
            let opts =
                crate::status_icon_themes::options(themes, &config.left_dock.status_icon_theme);
            let current =
                SharedString::from(config.left_dock.status_icon_theme.as_str().to_owned());
            select::state_with_options(opts, Some(&current), window, cx)
        });
        let file_icon_color_select = cx.new(|cx| {
            let opts = vec![
                select::SelectOption::new(ICON_COLOR, s::settings::icon_color_color()),
                select::SelectOption::new(ICON_MONOCHROME, s::settings::icon_color_monochrome()),
            ];
            let current =
                SharedString::new_static(icon_color_value(&config.left_dock.file_icon_color_mode));
            select::state_with_options(opts, Some(&current), window, cx)
        });
        Self {
            editor_select,
            left_dock_size_select,
            status_icon_theme_select,
            file_icon_color_select,
        }
    }
}

#[cfg(test)]
mod tests;
