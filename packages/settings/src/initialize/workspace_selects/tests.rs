use super::*;

#[gpui::test]
fn empty_editor_preserves_the_system_default_selection(cx: &mut gpui::TestAppContext) {
    daruda_ui::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        let mut config = Config::default();
        config.editor.preferred.clear();
        let selects = WorkspaceSelects::new(&config, &[], window, cx);
        assert_eq!(
            selects
                .editor_select
                .read(cx)
                .selected_value()
                .as_ref()
                .map(|value| value.as_ref()),
            Some("")
        );
        gpui::Empty
    });
}
