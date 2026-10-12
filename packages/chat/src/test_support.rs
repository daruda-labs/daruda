//! The globals a chat test needs before a pane renders.

pub(crate) fn init_gpui_component(cx: &mut gpui::TestAppContext) {
    daruda_ui::test_support::init_gpui_component(cx);
}
