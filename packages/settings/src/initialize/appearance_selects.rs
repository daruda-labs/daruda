use super::*;

pub(super) struct AppearanceSelects {
    pub(super) language_select: Entity<SelectState>,
    pub(super) terminal_preset_select: Entity<SelectState>,
    pub(super) ui_preset_select: Entity<SelectState>,
    pub(super) terminal_font_family_select: Entity<SelectState>,
    pub(super) editor_font_family_select: Entity<SelectState>,
    pub(super) agent_chat_font_family_select: Entity<SelectState>,
}

impl AppearanceSelects {
    pub(super) fn new(config: &Config, window: &mut Window, cx: &mut gpui::App) -> Self {
        // Language select — options driven by the canonical locale list so
        // adding a new locale only requires updating SUPPORTED_LOCALES.
        let lang = SharedString::from(config.general.language.clone());
        let language_select = cx.new(|cx| {
            let opts: Vec<select::SelectOption> = daruda_config::SUPPORTED_LOCALES
                .iter()
                .map(|&slug| {
                    let label = match slug {
                        "auto" => s::settings::language_auto(),
                        "en" => s::settings::language_en(),
                        "ko" => s::settings::language_ko(),
                        other => other.to_owned(),
                    };
                    select::SelectOption::new(slug, label)
                })
                .collect();
            select::state_with_options(opts, Some(&lang), window, cx)
        });

        // Terminal preset select — cell palette (fg/bg + ANSI 16).
        let terminal_preset = SharedString::from(config.theme.terminal_preset.clone());
        let terminal_preset_select = cx.new(|cx| {
            let opts = daruda_config::THEME_PRESETS
                .iter()
                .map(|p| SelectOption::new(p.name, p.display_name))
                .collect();
            select::state_with_options(opts, Some(&terminal_preset), window, cx)
        });

        // UI preset select — chrome palette (workspace, modal, status bar, …).
        let ui_preset = SharedString::from(config.theme.ui_preset.clone());
        let ui_preset_select = cx.new(|cx| {
            let system = SelectOption::new(
                daruda_config::ui_theme_presets::SYSTEM,
                s::settings::ui_preset_system(),
            );
            let opts = std::iter::once(system)
                .chain(
                    daruda_config::UI_THEME_PRESETS
                        .iter()
                        .map(|p| SelectOption::new(p.name, p.display_name)),
                )
                .collect();
            select::state_with_options(opts, Some(&ui_preset), window, cx)
        });

        let terminal_font_options = font_select_options(cx, &[&config.font.terminal.family]);
        let editor_font_options = font_select_options(cx, &[&config.font.editor.family]);
        let agent_chat_font_options = font_select_options(cx, &[&config.font.agent_chat.family]);
        let terminal_font_family = SharedString::from(config.font.terminal.family.clone());
        let terminal_font_family_select = cx.new(|cx| {
            select::state_with_options(
                terminal_font_options,
                Some(&terminal_font_family),
                window,
                cx,
            )
        });
        let editor_font_family = SharedString::from(config.font.editor.family.clone());
        let editor_font_family_select = cx.new(|cx| {
            select::state_with_options(editor_font_options, Some(&editor_font_family), window, cx)
        });
        let agent_chat_font_family = SharedString::from(config.font.agent_chat.family.clone());
        let agent_chat_font_family_select = cx.new(|cx| {
            select::state_with_options(
                agent_chat_font_options,
                Some(&agent_chat_font_family),
                window,
                cx,
            )
        });

        Self {
            language_select,
            terminal_preset_select,
            ui_preset_select,
            terminal_font_family_select,
            editor_font_family_select,
            agent_chat_font_family_select,
        }
    }
}

#[cfg(test)]
mod tests;
