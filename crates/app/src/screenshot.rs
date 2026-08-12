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

/// CLI flag selecting the standalone-terminal widen-reflow repro capture.
const TERMINAL_WIDEN_FLAG: &str = "--screenshot-terminal-widen";

/// Holds the driven terminal view between the open-window closure and the
/// async capture driver. A global rather than a captured handle so the
/// `Send` async block can re-acquire it via `cx.global` on the main thread.
struct ScreenshotTerminal(gpui::Entity<daruda_terminal::view::TerminalView>);
impl gpui::Global for ScreenshotTerminal {}

/// `true` when `--screenshot-terminal-widen` is present. This drives the
/// widen-reflow scrollback-dedup repro: it needs a terminal with
/// soft-wrapped scrollback and a real narrow→wide resize, neither of which
/// the restored-workspace capture path can produce.
pub(crate) fn parse_terminal_widen_flag() -> bool {
    std::env::args().any(|a| a == TERMINAL_WIDEN_FLAG)
}

/// Open a standalone `TerminalView` and drive it through real
/// narrow↔wide reflows to capture the scrollback-dedup behavior at the
/// pixel surface. Writes two PNGs — `<base>.widen.png` (narrow→wide, the
/// re-entry-duplication case) and `<base>.narrow.png` (wide→narrow).
///
/// Two facts shape the sequence:
///   - **Headless only paints at capture**, so window resizes never run
///     `resize_to_fit` mid-run. We drive the grid via `resize_terminal`,
///     which calls `TerminalSession::resize` immediately — the real reflow
///     + dedup path, no paint required.
///   - **The reflow seam forms only while the viewport is shorter than the
///     content.** So we feed and reflow at a short grid (6 rows) to create
///     the seam, then grow the grid tall purely to bring the whole unified
///     frame on screen for the capture. Reflowing straight into a tall grid
///     would let every row fit, dissolving the seam before it is visible.
///
/// Each capture also prints a one-line marker-integrity summary (unique
/// count + any duplicates), so the run self-reports pass/fail next to the
/// PNG. With the dedup live both cases read 30 unique, 0 duplicated.
pub(crate) fn schedule_terminal_widen_capture(path: PathBuf, cx: &mut App) {
    use gpui::{Bounds, Point, WindowBounds, WindowOptions};

    let session = match daruda_terminal::TerminalSession::new(
        daruda_terminal::TerminalDims::default(),
        daruda_terminal::TerminalConfig::default(),
    ) {
        Ok(s) => s,
        Err(e) => {
            println!("terminal-widen capture: session init failed: {e}");
            cx.quit();
            return;
        }
    };

    // Capture-window geometry (harness fixtures, not UI theme values).
    let (origin, win_w, win_h) = (80.0_f32, 1180.0_f32, 620.0_f32);
    let opts = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Point::new(px(origin), px(origin)),
            size(px(win_w), px(win_h)),
        ))),
        ..Default::default()
    };

    let opened = cx.open_window(opts, |window, cx| {
        let view = cx.new(|cx| {
            let focus = cx.focus_handle();
            daruda_terminal::view::TerminalView::new(session, focus)
        });
        cx.set_global(ScreenshotTerminal(view.clone()));
        cx.new(|cx| gpui_component::Root::new(view, window, cx))
    });
    let window = match opened {
        Ok(w) => w,
        Err(e) => {
            println!("terminal-widen capture: open_window failed: {e:#}");
            cx.quit();
            return;
        }
    };

    let widen_path = derive_path(&path, "widen");
    let narrow_path = derive_path(&path, "narrow");

    cx.spawn(async move |cx| {
        let grid = |cx: &mut gpui::AsyncApp, cols: u16, rows: u16| {
            cx.update(|cx| {
                let view = cx.global::<ScreenshotTerminal>().0.clone();
                view.update(cx, |v, cx| v.resize_terminal(cols, rows, cx));
            });
        };
        let feed = |cx: &mut gpui::AsyncApp, bytes: Vec<u8>| {
            cx.update(|cx| {
                let view = cx.global::<ScreenshotTerminal>().0.clone();
                view.update(cx, |v, cx| v.feed_output_bytes(&bytes, cx));
            });
        };
        // ~156 chars so each line wraps to 2 rows at the narrow width
        // (≈80 cols) and collapses to 1 row at the wide width (≈220 cols).
        let lines = |range: std::ops::RangeInclusive<u32>| -> Vec<u8> {
            let mut out = Vec::new();
            for i in range {
                out.extend_from_slice(format!("MK{i:02} {}\r\n", "=".repeat(150)).as_bytes());
            }
            out
        };
        let settle =
            |cx: &mut gpui::AsyncApp| cx.background_executor().timer(Duration::from_millis(450));
        // Position the viewport across the scrollback↔live seam: the top
        // half shows LineBuffer (scrollback) rows, the bottom half the live
        // grid. A re-entry duplication shows the same block in both halves.
        let scroll_seam = |cx: &mut gpui::AsyncApp| {
            cx.update(|cx| {
                let view = cx.global::<ScreenshotTerminal>().0.clone();
                view.update(cx, |v, cx| v.scroll_lines_into_history(8, cx));
            });
        };
        // Scan the whole unified frame and report `MKnn` marker counts: a
        // duplicated marker means the dedup let a re-entry through, a missing
        // one means a line was lost.
        let report = |cx: &mut gpui::AsyncApp, tag: &str| {
            cx.update(|cx| {
                let view = cx.global::<ScreenshotTerminal>().0.clone();
                let v = view.read(cx);
                let s = v.session();
                let mut counts: std::collections::BTreeMap<String, u32> = Default::default();
                for y in 0..s.total_rows() {
                    let t = s.dump_screen_row(y).unwrap_or_default();
                    if let Some(mk) = t.get(0..4).filter(|mk| mk.starts_with("MK")) {
                        *counts.entry(mk.to_string()).or_default() += 1;
                    }
                }
                let dups: Vec<_> = counts
                    .iter()
                    .filter(|&(_, &c)| c > 1)
                    .map(|(k, c)| format!("{k}×{c}"))
                    .collect();
                println!(
                    "[{tag}] unique markers={} duplicated=[{}]",
                    counts.len(),
                    dups.join(", ")
                );
            });
        };

        settle(cx).await;

        // ---- WIDEN: feed at narrow+short (lines wrap, several scroll into
        //      the LineBuffer) → widen at the same short height (re-entry
        //      seam) → grow tall to bring the whole frame on screen.
        grid(cx, 80, 6);
        feed(cx, lines(1..=30));
        grid(cx, 220, 6);
        grid(cx, 220, 16);
        scroll_seam(cx);
        settle(cx).await;
        report(cx, "widen");
        match cx.update(|cx| capture_window(Some(window.into()), &widen_path, cx)) {
            Ok(()) => println!("screenshot written: {}", widen_path.display()),
            Err(e) => println!("screenshot failed: {e:#}"),
        }

        // ---- NARROW: reset (RIS) → feed at wide+short (lines fit one row) →
        //      narrow at the same short height (lines re-wrap) → grow tall.
        feed(cx, b"\x1bc".to_vec());
        grid(cx, 220, 6);
        feed(cx, lines(1..=30));
        grid(cx, 80, 6);
        grid(cx, 80, 16);
        scroll_seam(cx);
        settle(cx).await;
        report(cx, "narrow");
        match cx.update(|cx| capture_window(Some(window.into()), &narrow_path, cx)) {
            Ok(()) => println!("screenshot written: {}", narrow_path.display()),
            Err(e) => println!("screenshot failed: {e:#}"),
        }

        // ---- ROUND-TRIP: reset → feed wide+short → narrow (shrink) → widen
        //      back tall (grow), ending wide. The realistic window-drag path;
        //      ending wide means no straddler at the viewport top, so the
        //      dedup keeps every line exactly once.
        feed(cx, b"\x1bc".to_vec());
        grid(cx, 220, 6);
        feed(cx, lines(1..=30));
        grid(cx, 80, 6); // shrink
        grid(cx, 220, 16); // grow back, ending wide
        scroll_seam(cx);
        settle(cx).await;
        report(cx, "roundtrip");
        let roundtrip_path = derive_path(&path, "roundtrip");
        match cx.update(|cx| capture_window(Some(window.into()), &roundtrip_path, cx)) {
            Ok(()) => println!("screenshot written: {}", roundtrip_path.display()),
            Err(e) => println!("screenshot failed: {e:#}"),
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
        | ScreenshotScenario::Toast
        | ScreenshotScenario::PaneContextMenu
        | ScreenshotScenario::MermaidLightbox
        | ScreenshotScenario::FlowPicker
        | ScreenshotScenario::FlowProfilePicker
        | ScreenshotScenario::FlowResumable
        | ScreenshotScenario::FlowRunning
        | ScreenshotScenario::FlowAsking => None,
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
