use super::*;

#[gpui::test]
fn off_list_fonts_are_selected_independently(cx: &mut gpui::TestAppContext) {
    daruda_ui::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        let mut config = Config::default();
        config.font.terminal.family = "Daruda off-list terminal".into();
        config.font.editor.family = "Daruda off-list editor".into();
        config.font.agent_chat.family = daruda_config::SYSTEM_UI_FONT_FAMILY.into();
        let selects = AppearanceSelects::new(&config, window, cx);
        for (state, expected) in [
            (
                &selects.terminal_font_family_select,
                config.font.terminal.family.as_str(),
            ),
            (
                &selects.editor_font_family_select,
                config.font.editor.family.as_str(),
            ),
            (
                &selects.agent_chat_font_family_select,
                config.font.agent_chat.family.as_str(),
            ),
        ] {
            assert_eq!(
                state
                    .read(cx)
                    .selected_value()
                    .as_ref()
                    .map(|value| value.as_ref()),
                Some(expected)
            );
        }
        gpui::Empty
    });
}
