use super::*;

#[gpui::test]
fn keep_awake_modes_have_distinct_selections(cx: &mut gpui::TestAppContext) {
    use daruda_config::KeepAwakeMode::{Agent, Off, On};

    daruda_ui::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        for (mode, expected) in [(Off, "off"), (Agent, "agent"), (On, "on")] {
            let mut config = Config::default();
            config.desktop.keep_awake = mode;
            let selects = BehaviorSelects::new(&config, window, cx);
            assert_eq!(
                selects
                    .keep_awake_mode_select
                    .read(cx)
                    .selected_value()
                    .as_ref()
                    .map(|value| value.as_ref()),
                Some(expected),
                "{mode:?}"
            );
        }
        gpui::Empty
    });
}
