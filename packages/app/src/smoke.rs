//! `daruda --smoke` — bring the real window up, let it paint, exit 0.
//!
//! The question a Windows CI job cannot otherwise answer: does gpui come up
//! on a runner with no GPU, where D3D11 falls back to software? A build and
//! a unit test never touch a swapchain. Not behind `screenshot`, which pulls
//! in `test-support` and is off in the binary an installer ships.

use std::time::Duration;

use gpui::{App, AppContext as _, Bounds, Pixels, px};

/// CLI flag that selects smoke mode.
const SMOKE_FLAG: &str = "--smoke";

/// Frames to draw before reading the window back. The window handle is
/// already registered by the time this runs — `open_first_window` is
/// synchronous — so nothing here waits for one to appear; this waits for it
/// to be *drawn*, which is the part a software renderer can fail at.
const PAINT: Duration = Duration::from_secs(2);

/// `true` when `--smoke` is present.
pub(crate) fn requested() -> bool {
    parse_from(std::env::args())
}

fn parse_from(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|arg| arg == SMOKE_FLAG)
}

/// Wait for a window, let it paint, then report and quit.
///
/// A window that is there and has area is as far as this goes: gpui exposes no
/// frame counter, so the rest of the evidence is negative — the process
/// survived the frames drawn during [`PAINT`], and a renderer that fails
/// mid-paint panics, which the exit code carries.
pub(crate) fn schedule(cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(PAINT).await;

        let Some(window) = cx.update(|cx| cx.windows().into_iter().next()) else {
            fail("no window is open");
        };

        match cx.update(|cx| cx.update_window(window, |_, window, _| window.bounds())) {
            Ok(bounds) if has_area(bounds) => {
                println!("smoke ok: window {:?}", bounds.size);
                cx.update(|cx| cx.quit());
            }
            Ok(bounds) => fail(&format!("window has no area: {:?}", bounds.size)),
            Err(e) => fail(&format!("window went away before it could be read: {e}")),
        }
    })
    .detach();
}

fn has_area(bounds: Bounds<Pixels>) -> bool {
    bounds.size.width > px(0.) && bounds.size.height > px(0.)
}

/// Report and leave with a code CI reads. Never `quit`, which exits 0 — a
/// smoke check that failed must not look like one that passed.
fn fail(reason: &str) -> ! {
    println!("smoke failed: {reason}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> bool {
        parse_from(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn the_flag_is_recognized_anywhere_in_the_line() {
        assert!(parse(&["daruda", "--smoke"]));
        assert!(parse(&["daruda", "--smoke", "extra"]));
    }

    #[test]
    fn an_ordinary_launch_is_not_smoke_mode() {
        assert!(!parse(&["daruda"]));
        assert!(!parse(&["daruda", "--screenshot", "/tmp/a.png"]));
    }

    /// Nothing else may opt in by accident — the check quits the app, so a
    /// prefix match would end an ordinary launch two seconds in.
    #[test]
    fn a_longer_flag_that_starts_the_same_is_not_smoke_mode() {
        assert!(!parse(&["daruda", "--smoke-test"]));
        assert!(!parse(&["daruda", "--smoke=1"]));
    }

    #[test]
    fn a_window_with_no_area_is_not_a_pass() {
        let sized = Bounds {
            origin: gpui::point(px(0.), px(0.)),
            size: gpui::size(px(800.), px(600.)),
        };
        let empty = Bounds {
            origin: gpui::point(px(0.), px(0.)),
            size: gpui::size(px(0.), px(0.)),
        };
        assert!(has_area(sized));
        assert!(!has_area(empty));
    }
}
