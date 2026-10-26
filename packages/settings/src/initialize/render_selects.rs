use super::*;

pub(super) struct RenderSelects {
    pub(super) max_fps_select: Entity<SelectState>,
    pub(super) syntax_theme_select: Entity<SelectState>,
}

impl RenderSelects {
    pub(super) fn new(config: &Config, window: &mut Window, cx: &mut gpui::App) -> Self {
        let max_fps_str: SharedString = config.render.max_fps.to_string().into();
        let max_fps_select = cx.new(|cx| {
            let opts = daruda_config::ALLOWED_MAX_FPS
                .iter()
                .map(|fps| {
                    SelectOption::new(
                        SharedString::from(fps.to_string()),
                        s::settings::max_fps_option(*fps),
                    )
                })
                .collect();
            select::state_with_options(opts, Some(&max_fps_str), window, cx)
        });

        // Resolve through the palette so a legacy / unknown stored value
        // (e.g. an old "base16-ocean.dark") still shows its effective
        // palette selected instead of a blank dropdown.
        let syntax_theme = SharedString::from(
            daruda_ui::theme::SyntaxPalette::from_config_name(&config.file_viewer.syntax_theme)
                .config_name(),
        );
        let syntax_theme_select = cx.new(|cx| {
            let opts = SYNTAX_THEMES
                .iter()
                .map(|v| SelectOption::new(*v, syntax_theme_label(v)))
                .collect();
            select::state_with_options(opts, Some(&syntax_theme), window, cx)
        });

        Self {
            max_fps_select,
            syntax_theme_select,
        }
    }
}

#[cfg(test)]
mod tests;
