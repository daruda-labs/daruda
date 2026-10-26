use super::*;

#[gpui::test]
fn keep_awake_modes_have_distinct_selections(cx: &mut gpui::TestAppContext) {
    daruda_ui::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        let mut config = Config::default();
        config.desktop.keep_awake = daruda_config::KeepAwakeMode::Agent;
        let selects = BehaviorSelects::new(&config, window, cx);
        assert_eq!(
            selects
                .keep_awake_mode_select
                .read(cx)
                .selected_value()
                .as_ref()
                .map(|value| value.as_ref()),
            Some(crate::spec::keep_awake_option(config.desktop.keep_awake))
        );
        gpui::Empty
    });
}
