//! `daruda --screenshot <path>` — render the live workspace window to a PNG
//! via gpui's permission-free `render_to_image` (offscreen Metal capture),
//! then quit. This is the automation entry point for visual verification:
//! render the real app → PNG → an agent reads the PNG back.
//!
//! Unlike `--hook` (a non-GUI subcommand handled before the app is built),
//! this needs the full GUI. It runs inside `app.run`: after the first window
//! opens it waits a short settle interval (async project/git/terminal restore
//! plus the first frames), captures the window, writes the PNG, and quits.
//!
//! Accepts `--screenshot <path>` and `--screenshot=<path>`. The capture target
//! is the first open window — the restored workspace or the welcome screen,
//! i.e. whatever the user would see on launch.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result};
use gpui::{App, AppContext as _};

/// CLI flag that selects screenshot mode.
const SCREENSHOT_FLAG: &str = "--screenshot";

/// How long to let the workspace settle (async restore + first frames) before
/// capturing. Generous enough for project/git load on a cold start; tune via a
/// follow-up env override if CI machines need longer.
const SETTLE_DELAY: Duration = Duration::from_millis(2000);

/// Parse `--screenshot <path>` / `--screenshot=<path>` from the process args.
/// Returns the target PNG path when screenshot mode is requested.
pub(crate) fn parse_screenshot_arg() -> Option<PathBuf> {
    parse_from(std::env::args())
}

fn parse_from(mut args: impl Iterator<Item = String>) -> Option<PathBuf> {
    while let Some(arg) = args.next() {
        if let Some(path) = arg.strip_prefix(concat!("--screenshot", "=")) {
            return Some(PathBuf::from(path));
        }
        if arg == SCREENSHOT_FLAG {
            return args.next().map(PathBuf::from);
        }
    }
    None
}

/// Schedule a one-shot capture of the first window, then quit the app. Call
/// from inside `app.run` after the first window has been opened.
pub(crate) fn schedule_capture(path: PathBuf, cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(SETTLE_DELAY).await;
        // AsyncApp::update is infallible; the inner Result is the capture's.
        let outcome = cx.update(|cx| capture_first_window(&path, cx));
        match outcome {
            Ok(()) => println!("screenshot written: {}", path.display()),
            Err(error) => println!("screenshot failed: {error:#}"),
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// Render the first open window to `path` as a PNG.
fn capture_first_window(path: &Path, cx: &mut App) -> Result<()> {
    let window = cx
        .windows()
        .into_iter()
        .next()
        .context("no open window to capture")?;
    let image = cx
        .update_window(window, |_, window, _| window.render_to_image())
        .context("capture window is gone")??;
    image.save(path).context("write screenshot png")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Option<PathBuf> {
        parse_from(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn space_separated_form() {
        assert_eq!(
            parse(&["daruda", "--screenshot", "/tmp/a.png"]),
            Some(PathBuf::from("/tmp/a.png"))
        );
    }

    #[test]
    fn equals_form() {
        assert_eq!(
            parse(&["daruda", "--screenshot=/tmp/b.png"]),
            Some(PathBuf::from("/tmp/b.png"))
        );
    }

    #[test]
    fn absent_or_unrelated() {
        assert_eq!(parse(&["daruda"]), None);
        assert_eq!(parse(&["daruda", "--other", "x"]), None);
    }

    #[test]
    fn flag_without_value() {
        assert_eq!(parse(&["daruda", "--screenshot"]), None);
    }
}
