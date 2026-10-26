use super::*;

#[gpui::test]
fn unknown_syntax_theme_selects_its_effective_palette(cx: &mut gpui::TestAppContext) {
    daruda_ui::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        let mut config = Config::default();
        config.file_viewer.syntax_theme = "removed-syntax-theme".into();
        let selects = RenderSelects::new(&config, window, cx);
        let effective =
            daruda_ui::theme::SyntaxPalette::from_config_name(&config.file_viewer.syntax_theme)
                .config_name();
        assert_eq!(
            selects
                .syntax_theme_select
                .read(cx)
                .selected_value()
                .as_ref()
                .map(|value| value.as_ref()),
            Some(effective)
        );
        gpui::Empty
    });
}
