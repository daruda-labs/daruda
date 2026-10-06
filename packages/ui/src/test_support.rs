//! The globals a widget test needs before it renders anything.

use gpui::TestAppContext;

pub(crate) fn init_gpui_component(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        // DarudaTheme must be registered before apply_daruda_palette, which
        // reads it to map slots into `gpui_component::Theme`.
        crate::theme::DarudaTheme::init(cx);
        crate::theme::apply_daruda_palette(cx);
    });
}
