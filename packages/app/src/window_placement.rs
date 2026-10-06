//! Where a workspace window goes: the geometry kept for it while it runs,
//! and where a saved one reopens.

use gpui::{App, Bounds, Point, Size, WindowBounds, px};

/// Where a saved window reopens: on the display it was on when that display
/// is still attached, else the primary one, and moved wholly onto it — a
/// rect saved on a monitor since unplugged or a larger one would open off
/// screen. gpui reads the rect relative to the display it opens on.
pub(crate) fn restored_placement(
    saved: &daruda_store::project::WindowState,
    cx: &App,
) -> (WindowBounds, Option<gpui::DisplayId>) {
    let display = saved
        .display
        .as_deref()
        .and_then(|want| {
            cx.displays()
                .into_iter()
                .find(|d| d.uuid().ok().is_some_and(|uuid| uuid.to_string() == want))
        })
        .or_else(|| cx.primary_display());
    let fitted = match &display {
        Some(d) => {
            let area = d.bounds();
            saved.fit_within(
                f32::from(area.origin.x),
                f32::from(area.origin.y),
                f32::from(area.size.width),
                f32::from(area.size.height),
            )
        }
        None => saved.clone(),
    };
    let rect = Bounds::new(
        Point::new(px(fitted.x), px(fitted.y)),
        Size::new(px(fitted.width), px(fitted.height)),
    );
    let bounds = if fitted.maximized {
        WindowBounds::Maximized(rect)
    } else {
        WindowBounds::Windowed(rect)
    };
    (bounds, display.map(|d| d.id()))
}

/// The geometry to keep for a window reporting `reported`. While it is
/// maximized the rect worth keeping is the one it restores to, which gpui
/// does not report alike everywhere — macOS gives the maximized frame as
/// windowed, X11 gives it as maximized — so the last windowed rect is kept
/// and only the flag and display change.
pub(crate) fn captured_geometry(
    reported: daruda_store::project::WindowState,
    maximized: bool,
    previous: Option<&daruda_store::project::WindowState>,
) -> daruda_store::project::WindowState {
    match previous {
        Some(prev) if maximized => daruda_store::project::WindowState {
            maximized: true,
            display: reported.display,
            ..prev.clone()
        },
        _ => daruda_store::project::WindowState {
            maximized,
            ..reported
        },
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::captured_geometry;
    use daruda_store::project::WindowState;

    fn rect(x: f32, width: f32) -> WindowState {
        WindowState {
            x,
            y: 0.0,
            width,
            height: 600.0,
            ..Default::default()
        }
    }

    #[test]
    fn a_windowed_window_keeps_what_it_reports() {
        let kept = captured_geometry(rect(10.0, 800.0), false, Some(&rect(0.0, 500.0)));
        assert_eq!((kept.x, kept.width, kept.maximized), (10.0, 800.0, false));
    }

    #[test]
    fn a_maximized_window_keeps_the_rect_it_restores_to() {
        let kept = captured_geometry(rect(0.0, 1920.0), true, Some(&rect(10.0, 800.0)));
        assert_eq!((kept.x, kept.width, kept.maximized), (10.0, 800.0, true));
    }

    #[test]
    fn a_window_maximized_from_the_start_keeps_its_reported_rect() {
        let kept = captured_geometry(rect(0.0, 1920.0), true, None);
        assert_eq!((kept.width, kept.maximized), (1920.0, true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::project::WindowState;

    fn saved(x: f32, y: f32, display: Option<String>, maximized: bool) -> WindowState {
        WindowState {
            x,
            y,
            width: 800.0,
            height: 600.0,
            display,
            maximized,
        }
    }

    fn rect(bounds: &WindowBounds) -> (f32, f32, f32, f32) {
        let b = match bounds {
            WindowBounds::Windowed(b)
            | WindowBounds::Maximized(b)
            | WindowBounds::Fullscreen(b) => b,
        };
        (
            f32::from(b.origin.x),
            f32::from(b.origin.y),
            f32::from(b.size.width),
            f32::from(b.size.height),
        )
    }

    #[gpui::test]
    fn a_window_saved_on_a_display_now_gone_reopens_on_the_primary(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let primary = cx.primary_display().expect("test platform has a display");
            let (bounds, display) =
                restored_placement(&saved(3000.0, 50.0, Some("unplugged".into()), false), cx);
            assert_eq!(display, Some(primary.id()));
            assert_eq!(rect(&bounds), (1120.0, 50.0, 800.0, 600.0), "moved onto it");
        });
    }

    #[gpui::test]
    fn a_window_on_its_own_display_keeps_its_place(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let primary = cx.primary_display().unwrap();
            let uuid = primary.uuid().unwrap().to_string();
            let (bounds, display) = restored_placement(&saved(100.0, 80.0, Some(uuid), false), cx);
            assert_eq!(display, Some(primary.id()));
            assert!(matches!(bounds, WindowBounds::Windowed(_)));
            assert_eq!(rect(&bounds), (100.0, 80.0, 800.0, 600.0));
        });
    }

    #[gpui::test]
    fn a_maximized_window_reopens_maximized(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let (bounds, _) = restored_placement(&saved(100.0, 80.0, None, true), cx);
            assert!(matches!(bounds, WindowBounds::Maximized(_)));
            assert_eq!(
                rect(&bounds),
                (100.0, 80.0, 800.0, 600.0),
                "the rect it restores to"
            );
        });
    }
}
