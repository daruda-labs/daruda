use super::*;
use gpui::{
    Context, IntoElement, Render, RenderOnce, TestAppContext, VisualTestContext, Window, div,
    prelude::*,
};
use std::{cell::Cell, rc::Rc};

#[derive(IntoElement)]
struct TextMetrics(Rc<Cell<(f32, f32)>>);

impl RenderOnce for TextMetrics {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let style = window.text_style();
        self.0.set((
            style.font_size.to_pixels(window.rem_size()).into(),
            style
                .line_height
                .to_pixels(style.font_size, window.rem_size())
                .into(),
        ));
        div().child("Probe")
    }
}

struct Probe {
    rem: f32,
    standard: Rc<Cell<(f32, f32)>>,
    compact: Rc<Cell<(f32, f32)>>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(px(self.rem));
        div()
            .flex()
            .flex_col()
            .items_start()
            .text_size(px(32.))
            .line_height(px(40.))
            .child(button("standard", "Save").child(TextMetrics(self.standard.clone())))
            .child(button_compact("compact", "Fetch").child(TextMetrics(self.compact.clone())))
            .child(
                button_with_icon("labelled", "New", crate::icons::ADD)
                    .debug_selector(|| "labelled-action".into()),
            )
            .child(
                button_bare_compact("icon")
                    .icon(crate::Icon::new(crate::IconName::Plus))
                    .debug_selector(|| "compact-icon".into()),
            )
            .child(
                compact_icon_button(crate::copy_button(
                    "copy",
                    "value".into(),
                    crate::icons::icon(crate::icons::COPY),
                    "Copy".into(),
                    crate::icons::icon(crate::icons::CHECK),
                    "Copied".into(),
                    window,
                    cx,
                ))
                .debug_selector(|| "compact-copy".into()),
            )
    }
}

#[gpui::test]
fn button_text_and_icon_targets_are_pixel_sized(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    for rem in [16., 20., 24.] {
        let standard = Rc::new(Cell::new((0., 0.)));
        let compact = Rc::new(Cell::new((0., 0.)));
        let standard_probe = standard.clone();
        let compact_probe = compact.clone();
        let window = cx.add_window(move |_, _| Probe {
            rem,
            standard: standard_probe,
            compact: compact_probe,
        });
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        assert_eq!(standard.get(), (12., 20.));
        assert_eq!(compact.get(), (12., 16.));
        assert_eq!(
            vcx.debug_bounds("labelled-action").unwrap().size.height,
            px(28.)
        );
        for selector in ["compact-icon", "compact-copy"] {
            assert_eq!(
                vcx.debug_bounds(selector).unwrap().size,
                gpui::size(px(20.), px(20.))
            );
        }
    }
}
