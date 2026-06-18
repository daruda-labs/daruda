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
use gpui::{AnyWindowHandle, App, AppContext as _, Pixels, Size, px, size};

use crate::workspace::screenshot_scenario::ScreenshotScenario;

/// Env var overriding the post-launch settle delay (milliseconds).
const SETTLE_ENV: &str = "DARUDA_SCREENSHOT_SETTLE_MS";

/// CLI flag fixing the captured window size, e.g. `--screenshot-size 1280x800`.
const SIZE_FLAG: &str = "--screenshot-size";

/// CLI flag that selects screenshot mode.
const SCREENSHOT_FLAG: &str = "--screenshot";

/// CLI flag that drives a transient overlay into view before capture.
const SCENARIO_FLAG: &str = "--screenshot-scenario";

/// CLI flag that overrides the UI theme before capture, orthogonal to the
/// scenario (e.g. `--screenshot-scenario command-palette --screenshot-theme light`).
const THEME_FLAG: &str = "--screenshot-theme";

/// UI theme override for a capture. Maps to a bundled `ui_preset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenshotTheme {
    Light,
    Dark,
}

impl ScreenshotTheme {
    /// Map a CLI token (`light` / `dark`) to a theme. Unknown tokens → `None`.
    fn from_cli_name(name: &str) -> Option<Self> {
        match name {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    /// The bundled `ui_preset` name passed to `apply_ui_theme`.
    fn ui_preset_name(self) -> &'static str {
        match self {
            Self::Light => "daruda_light",
            Self::Dark => "daruda_dark",
        }
    }

    /// Short slug used as a filename suffix in batch (multi-theme) captures.
    fn slug(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

/// How long to let the workspace settle (async restore + first frames) before
/// capturing. Generous enough for project/git load on a cold start; tune via a
/// follow-up env override if CI machines need longer.
const SETTLE_DELAY: Duration = Duration::from_millis(2000);

/// How long to let a driven scenario (modal animation, palette layout) render
/// after [`apply_scenario`] before capturing.
const SCENARIO_RENDER_DELAY: Duration = Duration::from_millis(400);

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

/// Parse `--screenshot-scenario <name>` / `--screenshot-scenario=<name>` from
/// the process args. Returns the scenario when a known one is requested.
pub(crate) fn parse_scenario_arg() -> Option<ScreenshotScenario> {
    parse_scenario_from(std::env::args())
}

fn parse_scenario_from(mut args: impl Iterator<Item = String>) -> Option<ScreenshotScenario> {
    while let Some(arg) = args.next() {
        if let Some(name) = arg.strip_prefix(concat!("--screenshot-scenario", "=")) {
            return ScreenshotScenario::from_cli_name(name);
        }
        if arg == SCENARIO_FLAG {
            return args
                .next()
                .as_deref()
                .and_then(ScreenshotScenario::from_cli_name);
        }
    }
    None
}

/// Parse `--screenshot-theme <light|dark>` / `=<…>` from the process args.
/// Accepts a comma-separated list (`light,dark`) for a batch capture — one
/// PNG per theme in a single launch. Unknown tokens are dropped; absent → empty.
pub(crate) fn parse_themes_arg() -> Vec<ScreenshotTheme> {
    parse_themes_from(std::env::args())
}

fn parse_themes_from(mut args: impl Iterator<Item = String>) -> Vec<ScreenshotTheme> {
    while let Some(arg) = args.next() {
        let value = if let Some(v) = arg.strip_prefix(concat!("--screenshot-theme", "=")) {
            Some(v.to_string())
        } else if arg == THEME_FLAG {
            args.next()
        } else {
            None
        };
        if let Some(value) = value {
            return value
                .split(',')
                .filter_map(|t| ScreenshotTheme::from_cli_name(t.trim()))
                .collect();
        }
    }
    Vec::new()
}

/// Resolve the settle delay from an optional env value (milliseconds),
/// falling back to [`SETTLE_DELAY`] when absent or unparseable.
fn settle_delay_from(env: Option<&str>) -> Duration {
    env.and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(SETTLE_DELAY)
}

/// Parse a `WxH` size string (e.g. `1280x800`) into a pixel size. Both
/// dimensions must be positive integers.
fn parse_size_str(s: &str) -> Option<Size<Pixels>> {
    let (w, h) = s.split_once('x')?;
    let w: f32 = w.trim().parse().ok()?;
    let h: f32 = h.trim().parse().ok()?;
    (w > 0.0 && h > 0.0).then(|| size(px(w), px(h)))
}

/// Parse `--screenshot-size WxH` / `=WxH` from the process args.
pub(crate) fn parse_size_arg() -> Option<Size<Pixels>> {
    parse_size_from(std::env::args())
}

fn parse_size_from(mut args: impl Iterator<Item = String>) -> Option<Size<Pixels>> {
    while let Some(arg) = args.next() {
        if let Some(v) = arg.strip_prefix(concat!("--screenshot-size", "=")) {
            return parse_size_str(v);
        }
        if arg == SIZE_FLAG {
            return args.next().as_deref().and_then(parse_size_str);
        }
    }
    None
}

/// Insert `suffix` before the extension of `base` (e.g. `shot.png` + `light`
/// → `shot.light.png`). Used to name batch (multi-theme) captures.
fn derive_path(base: &Path, suffix: &str) -> PathBuf {
    let stem = base.file_stem().map(|s| s.to_string_lossy().into_owned());
    let ext = base.extension().map(|e| e.to_string_lossy().into_owned());
    let name = match (stem, ext) {
        (Some(stem), Some(ext)) => format!("{stem}.{suffix}.{ext}"),
        (Some(stem), None) => format!("{stem}.{suffix}"),
        _ => return base.to_path_buf(),
    };
    base.with_file_name(name)
}

/// Schedule a one-shot capture of the first window, then quit the app. Call
/// from inside `app.run` after the first window has been opened.
pub(crate) fn schedule_capture(
    path: PathBuf,
    scenario: Option<ScreenshotScenario>,
    themes: Vec<ScreenshotTheme>,
    win_size: Option<Size<Pixels>>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let settle = settle_delay_from(std::env::var(SETTLE_ENV).ok().as_deref());
        cx.background_executor().timer(settle).await;

        // The scenario is applied once; the theme loop then re-themes the open
        // overlay in place (`apply_ui_theme` refreshes every window), so a batch
        // never needs to tear down and re-open the overlay between captures.
        let target = match scenario {
            Some(scenario) => cx.update(|cx| apply_scenario(scenario, cx)),
            None => None,
        };

        // No `--screenshot-theme` → one capture with whatever theme is live.
        let steps: Vec<Option<ScreenshotTheme>> = if themes.is_empty() {
            vec![None]
        } else {
            themes.iter().copied().map(Some).collect()
        };
        let batch = steps.len() > 1;

        for theme in steps {
            if let Some(theme) = theme {
                cx.update(|cx| {
                    if !crate::ui::theme::apply_ui_theme(theme.ui_preset_name(), cx) {
                        println!("screenshot theme not applied: {}", theme.ui_preset_name());
                    }
                });
            }
            if let Some(win_size) = win_size {
                cx.update(|cx| resize_target(target, win_size, cx));
            }
            // Let the theme swap / resize / scenario overlay paint before
            // capture (`render_to_image` reads the last painted frame).
            cx.background_executor().timer(SCENARIO_RENDER_DELAY).await;

            let out = match theme {
                Some(theme) if batch => derive_path(&path, theme.slug()),
                _ => path.clone(),
            };
            // AsyncApp::update is infallible; the inner Result is the capture's.
            let outcome = cx.update(|cx| capture_window(target, &out, cx));
            match outcome {
                Ok(()) => println!("screenshot written: {}", out.display()),
                Err(error) => println!("screenshot failed: {error:#}"),
            }
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// Resize the capture target window (or the first window when `None`) and
/// force a repaint so the new bounds are reflected in the captured frame.
fn resize_target(target: Option<AnyWindowHandle>, win_size: Size<Pixels>, cx: &mut App) {
    let Some(handle) = target.or_else(|| cx.windows().into_iter().next()) else {
        return;
    };
    crate::windows::try_update_workspace_window(
        handle,
        cx,
        "screenshot_resize",
        move |window, _| {
            window.resize(win_size);
            window.refresh();
        },
    );
}

/// Drive `scenario` into view on the workspace window, returning the window to
/// capture when the scenario opens its own (Settings); `None` falls back to the
/// first open window. Logs and skips when no workspace is open (e.g. the welcome
/// screen) so the capture still proceeds.
fn apply_scenario(scenario: ScreenshotScenario, cx: &mut App) -> Option<AnyWindowHandle> {
    let Some((handle, weak)) = crate::window_registry::WindowRegistry::first_workspace(cx) else {
        println!("screenshot scenario skipped: no workspace window");
        return None;
    };
    crate::windows::try_update_workspace_window(
        handle,
        cx,
        "screenshot_scenario",
        move |window, cx| {
            if let Some(workspace) = weak.upgrade() {
                crate::workspace::screenshot_scenario::drive(scenario, &workspace, window, cx);
            }
            // `render_to_image` captures the last *painted* frame, not a fresh
            // render. A bare `cx.notify` from the driver only marks the view
            // dirty; force a full repaint so the driven overlay lands in the
            // captured frame. This is a one-shot capture path, not the hot
            // path the `window.refresh()` ban targets.
            window.refresh();
        },
    );
    // Settings is a separate window; capture it instead of the workspace.
    match scenario {
        ScreenshotScenario::Settings(_) => {
            crate::window_registry::WindowRegistry::settings_window(cx)
        }
        ScreenshotScenario::CommandPalette
        | ScreenshotScenario::ErrorModal
        | ScreenshotScenario::Toast => None,
    }
}

/// Render `target` (or the first open window when `None`) to `path` as a PNG.
fn capture_window(target: Option<AnyWindowHandle>, path: &Path, cx: &mut App) -> Result<()> {
    let window = match target {
        Some(handle) => handle,
        None => cx
            .windows()
            .into_iter()
            .next()
            .context("no open window to capture")?,
    };
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

    fn parse_scenario(args: &[&str]) -> Option<ScreenshotScenario> {
        parse_scenario_from(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn scenario_space_separated_form() {
        assert_eq!(
            parse_scenario(&["daruda", "--screenshot-scenario", "command-palette"]),
            Some(ScreenshotScenario::CommandPalette)
        );
    }

    #[test]
    fn scenario_equals_form() {
        assert_eq!(
            parse_scenario(&["daruda", "--screenshot-scenario=error-modal"]),
            Some(ScreenshotScenario::ErrorModal)
        );
    }

    #[test]
    fn scenario_absent_or_unknown() {
        assert_eq!(parse_scenario(&["daruda"]), None);
        assert_eq!(
            parse_scenario(&["daruda", "--screenshot-scenario", "bogus"]),
            None
        );
    }

    #[test]
    fn scenario_flag_without_value() {
        assert_eq!(parse_scenario(&["daruda", "--screenshot-scenario"]), None);
    }

    fn parse_themes(args: &[&str]) -> Vec<ScreenshotTheme> {
        parse_themes_from(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn theme_space_and_equals_form() {
        assert_eq!(
            parse_themes(&["daruda", "--screenshot-theme", "light"]),
            vec![ScreenshotTheme::Light]
        );
        assert_eq!(
            parse_themes(&["daruda", "--screenshot-theme=dark"]),
            vec![ScreenshotTheme::Dark]
        );
    }

    #[test]
    fn theme_comma_list_is_a_batch() {
        assert_eq!(
            parse_themes(&["daruda", "--screenshot-theme", "light,dark"]),
            vec![ScreenshotTheme::Light, ScreenshotTheme::Dark]
        );
    }

    #[test]
    fn theme_absent_unknown_or_no_value_is_empty() {
        assert!(parse_themes(&["daruda"]).is_empty());
        assert!(parse_themes(&["daruda", "--screenshot-theme", "blue"]).is_empty());
        assert!(parse_themes(&["daruda", "--screenshot-theme"]).is_empty());
    }

    #[test]
    fn settle_delay_falls_back_when_absent_or_bad() {
        assert_eq!(settle_delay_from(None), SETTLE_DELAY);
        assert_eq!(settle_delay_from(Some("abc")), SETTLE_DELAY);
    }

    #[test]
    fn settle_delay_uses_env_milliseconds() {
        assert_eq!(settle_delay_from(Some("500")), Duration::from_millis(500));
    }

    #[test]
    fn size_str_parses_wxh() {
        assert_eq!(parse_size_str("1280x800"), Some(size(px(1280.), px(800.))));
    }

    #[test]
    fn size_str_rejects_bad_or_nonpositive() {
        assert_eq!(parse_size_str("nope"), None);
        assert_eq!(parse_size_str("1280"), None);
        assert_eq!(parse_size_str("0x800"), None);
    }

    fn parse_size(args: &[&str]) -> Option<Size<Pixels>> {
        parse_size_from(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn size_flag_space_and_equals_form() {
        assert_eq!(
            parse_size(&["daruda", "--screenshot-size", "1024x768"]),
            Some(size(px(1024.), px(768.)))
        );
        assert_eq!(
            parse_size(&["daruda", "--screenshot-size=640x480"]),
            Some(size(px(640.), px(480.)))
        );
    }

    #[test]
    fn derive_path_inserts_suffix_before_extension() {
        assert_eq!(
            derive_path(Path::new("/tmp/shot.png"), "light"),
            PathBuf::from("/tmp/shot.light.png")
        );
        assert_eq!(
            derive_path(Path::new("/tmp/shot"), "dark"),
            PathBuf::from("/tmp/shot.dark")
        );
    }
}
