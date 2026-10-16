use super::*;
use gpui::{HeadlessAppContext, Render};
use std::sync::Arc;

const PREVIEW_WIDTH: f32 = 160.0;
const PREVIEW_HEIGHT: f32 = 64.0;

struct Indicators {
    outcomes: [Option<AgentOutcome>; 2],
}

impl Render for Indicators {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .bg(theme::current(cx).dock_bg)
            .flex()
            .items_center()
            .justify_center()
            .gap(px(theme::GAP_LG))
            .children(self.outcomes.into_iter().map(|outcome| {
                div()
                    .w(px(theme::STATUS_INDICATOR_CELL_WIDTH))
                    .h(px(theme::STATUS_INDICATOR_SIZE))
                    .child(agent_status_cell(Some(SessionStatus::Idle), outcome, cx))
            }))
    }
}

#[test]
#[ignore = "requires a Metal renderer"]
fn capture_idle_and_completed_grids() {
    let mut app = HeadlessAppContext::with_platform(
        Arc::new(gpui::NoopTextSystem::new()),
        Arc::new(()),
        || {
            Some(Box::new(
                gpui_macos::metal_renderer::MetalHeadlessRenderer::new(),
            ))
        },
    );
    app.update(theme::DarudaTheme::init);
    let window = app
        .open_window(
            gpui::size(px(PREVIEW_WIDTH), px(PREVIEW_HEIGHT)),
            |_, cx| {
                cx.new(|_| Indicators {
                    outcomes: [None, Some(AgentOutcome::Completed)],
                })
            },
        )
        .unwrap();
    app.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
    let before = app.capture_screenshot(window.into()).unwrap();
    let green_pixels = |pixels: &[u8]| {
        pixels
            .chunks_exact(4)
            .filter(|pixel| {
                pixel[1] > pixel[0].saturating_add(20) && pixel[1] > pixel[2].saturating_add(10)
            })
            .count()
    };
    assert!(green_pixels(before.as_raw()) > 0);
    let view = window.root(&mut app).unwrap();
    view.update(&mut app, |view, cx| {
        view.outcomes[1] = None;
        cx.notify();
    });
    app.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
    let after = app.capture_screenshot(window.into()).unwrap();
    assert_eq!(green_pixels(after.as_raw()), 0);
    let output = tempfile::tempdir().unwrap().keep();
    before.save(output.join("idle-completed.png")).unwrap();
    after.save(output.join("acknowledged.png")).unwrap();
    println!("indicator captures: {}", output.display());
}
