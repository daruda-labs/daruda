//! Viewport sizing shared by anchored editors and status-bar panels.

use gpui::{Div, Pixels, Size, Window, div, prelude::*, px, size};

#[derive(Clone, Copy)]
pub struct PanelLimits {
    pub width: f32,
    pub max_height: f32,
    pub min_height: f32,
    pub margin: f32,
    /// Total horizontal/vertical space owned by the popover border and padding.
    pub chrome: f32,
}

impl PanelLimits {
    pub fn outer_size(self, viewport: Size<Pixels>, below: Option<Pixels>) -> Size<Pixels> {
        let available_w = (f32::from(viewport.width) - 2.0 * self.margin).max(0.0);
        let available_h = (f32::from(viewport.height) - 2.0 * self.margin).max(0.0);
        let room = below.map_or(available_h, |top| {
            (f32::from(viewport.height - top) - self.margin).max(0.0)
        });
        let floor = self.min_height.min(available_h);
        size(
            px(self.width.min(available_w).max(0.0)),
            px(self
                .max_height
                .min(room.max(floor))
                .min(available_h)
                .max(0.0)),
        )
    }
}

pub fn popover_panel(window: &Window, limits: PanelLimits, below: Option<Pixels>) -> Div {
    let outer = limits.outer_size(window.viewport_size(), below);
    div()
        .w(px((f32::from(outer.width) - limits.chrome).max(0.0)))
        .max_h(px((f32::from(outer.height) - limits.chrome).max(0.0)))
        .min_w_0()
        .overflow_hidden()
        .flex()
        .flex_col()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Bounds, Context, IntoElement, Render, TestAppContext, VisualTestContext, WindowBounds,
        WindowOptions, point,
    };

    struct Probe;

    impl Render for Probe {
        fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            popover_panel(
                window,
                PanelLimits {
                    width: 470.,
                    max_height: 640.,
                    min_height: 320.,
                    margin: 12.,
                    chrome: 2.,
                },
                None,
            )
            .debug_selector(|| "bounded-panel".into())
            .child(
                div()
                    .h(px(24.))
                    .flex_none()
                    .debug_selector(|| "panel-header".into()),
            )
            .child(
                crate::scroll_area(
                    "bounded-panel-body",
                    px(0.),
                    div()
                        .flex()
                        .flex_col()
                        .children((0..50).map(|_| div().h(px(20.)).flex_none())),
                )
                .fill(),
            )
            .child(
                div()
                    .h(px(24.))
                    .flex_none()
                    .debug_selector(|| "panel-footer".into()),
            )
        }
    }

    #[gpui::test]
    fn short_windows_keep_panel_bands_pinned_and_the_body_scrollable(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        let window = cx
            .update(|cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            point(px(0.), px(0.)),
                            size(px(320.), px(200.)),
                        ))),
                        ..Default::default()
                    },
                    |_, cx| cx.new(|_| Probe),
                )
            })
            .unwrap();
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let panel = vcx.debug_bounds("bounded-panel").unwrap();
        let header = vcx.debug_bounds("panel-header").unwrap();
        let footer = vcx.debug_bounds("panel-footer").unwrap();
        let body = vcx.debug_bounds("bounded-panel-body-wrapper").unwrap();
        assert!(panel.size.width <= px(294.));
        assert!(panel.size.height <= px(174.));
        assert_eq!(header.size.height, px(24.));
        assert_eq!(footer.size.height, px(24.));
        assert!(body.size.height > px(0.));
        assert_eq!(body.top(), header.bottom());
        assert_eq!(body.bottom(), footer.top());
        assert_eq!(footer.bottom(), panel.bottom());
        assert_eq!(
            vcx.debug_bounds("bounded-panel-body-scrollbar-layer")
                .unwrap(),
            body
        );
    }

    #[test]
    fn panels_fit_the_window_even_with_a_missing_or_offscreen_anchor() {
        let limits = PanelLimits {
            width: 470.,
            max_height: 640.,
            min_height: 320.,
            margin: 12.,
            chrome: 2.,
        };
        for (w, h) in [(1280., 900.), (320., 200.), (18., 18.)] {
            for top in [None, Some(px(100.)), Some(px(1400.))] {
                let actual = limits.outer_size(size(px(w), px(h)), top);
                assert!(actual.width >= px(0.) && actual.width <= px((w - 24.).max(0.)));
                assert!(actual.height >= px(0.) && actual.height <= px((h - 24.).max(0.)));
            }
        }
        assert_eq!(
            limits
                .outer_size(size(px(500.), px(500.)), Some(px(100.)))
                .height,
            px(388.)
        );
        assert_eq!(
            limits
                .outer_size(size(px(900.), px(900.)), Some(px(800.)))
                .height,
            px(320.)
        );
    }
}
