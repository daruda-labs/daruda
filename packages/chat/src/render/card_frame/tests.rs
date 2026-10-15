use super::*;

#[gpui::test]
fn failed_cards_are_readable_across_ui_and_terminal_presets(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        theme::init_if_missing(cx);
        for ui in ["daruda_light", "daruda_dark"] {
            assert!(theme::apply_ui_theme(ui, cx));
            for preset in daruda_config::theme_presets::PRESETS {
                let Some(colors) = daruda_config::theme_presets::colors_for_preset(preset.name)
                else {
                    continue;
                };
                let bg = colors.background;
                theme::set_agent_chat_bg(cx, bg.r, bg.g, bg.b);
                let color = CardFrame::new(CardTone::Error).status_color(0.0, cx);
                let background = theme::agent_chat_bg(cx).blend(theme::agent_chat_tint(cx));
                let ratio = theme::contrast_ratio(color, background);
                assert!(ratio >= 4.5, "{ui} / {}: {ratio:.2}:1", preset.name);
            }
            for (r, g, b) in [(250, 250, 250), (224, 228, 240)] {
                theme::set_agent_chat_bg(cx, r, g, b);
                let color = CardFrame::new(CardTone::Error).status_color(0.0, cx);
                let background = theme::agent_chat_bg(cx).blend(theme::agent_chat_tint(cx));
                let ratio = theme::contrast_ratio(color, background);
                assert!(ratio >= 4.5, "{ui} / light pane: {ratio:.2}:1");
            }
        }
    });
}
