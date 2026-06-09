//! Standalone visual verifier — renders a GPUI view through the real Metal
//! compositor and writes a PNG. Foundation for automated visual verification
//! of daruda's rendering: render → PNG → an agent reads the PNG back.
//!
//! Capture is permission-free: `capture_screenshot` calls
//! `Window::render_to_image` (zed PR #45259), which renders the scene to a
//! Metal texture and reads pixels directly — no ScreenCaptureKit, no
//! screen-recording grant. Must run on the main thread (macOS platform
//! requirement), so this is an `example` binary, not a `#[gpui::test]`:
//!
//!   cargo run -p daruda --example visual_verifier   # → /tmp/daruda_verifier.png
//!
//! Captures layout, colors, shapes, raster images (incl. rasterized mermaid
//! PNGs), AND text glyphs. Glyph rasterization requires the
//! `gpui_macos/font-kit` feature (enabled in the workspace manifest); without
//! it shapes render but text does not. The next step is rendering the real
//! markdown viewer through this path to verify text-bearing views.

use std::borrow::Cow;
use std::rc::Rc;
use std::time::Duration;

use gpui::AppContext as _;
use gpui::{
    Bounds, Context, IntoElement, ParentElement, Render, Styled, VisualTestAppContext, Window,
    WindowBounds, WindowOptions, div, point, px, rgb, size,
};

struct VerifierView;

impl Render for VerifierView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .text_xl()
            .text_color(rgb(0xffffff))
            .child("VERIFY TEXT 12345 ABCdef")
            .child(div().w(px(440.)).h(px(96.)).bg(rgb(0x89b4fa)))
            .child(div().w(px(220.)).h(px(44.)).bg(rgb(0xa6e3a1)))
            .child(div().w(px(220.)).h(px(44.)).bg(rgb(0xf38ba8)))
    }
}

fn main() {
    let platform = Rc::new(gpui_macos::MacPlatform::new(false));
    let mut cx = VisualTestAppContext::new(platform);

    // App-level setup before any window: init gpui_component (theme) and
    // register a font (gpui does not auto-load fonts).
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(include_bytes!(
                "../../../vendor/ghostty/src/font/res/JetBrainsMonoNoNF-Regular.ttf"
            ) as &[u8])])
            .expect("register font");
    });

    let window = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(0.), px(0.)),
                        size: size(px(1280.), px(800.)),
                    })),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |_window, cx| cx.new(|_| VerifierView),
            )
        })
        .expect("open window");

    // Stabilize before capture (mirrors zed's visual test runner): drain,
    // refresh, advance the simulated clock, drain again.
    let handle: gpui::AnyWindowHandle = window.into();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, _cx| window.refresh())
        .expect("refresh window");
    cx.advance_clock(Duration::from_millis(100));
    cx.run_until_parked();

    let img = cx.capture_screenshot(handle).expect("capture screenshot");
    let out = "/tmp/daruda_verifier.png";
    img.save(out).expect("save png");
    println!("wrote {out} ({}x{})", img.width(), img.height());
}
