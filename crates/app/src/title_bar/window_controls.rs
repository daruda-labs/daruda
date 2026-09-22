//! The drag strip and the minimize / maximize / close controls the app draws
//! when the platform left it no caption.

use gpui::{App, Div, MouseButton, Stateful, Window, WindowControlArea, div, prelude::*, px};

use super::policy::{ControlTier, WindowChrome};
use crate::ui::theme;

/// Box-drawing glyphs rather than SVG: daruda's asset source carries no
/// caption icons, and the dock toggles beside these already read as glyphs.
const GLYPH_MINIMIZE: &str = "\u{2500}";
const GLYPH_MAXIMIZE: &str = "\u{25a1}";
const GLYPH_RESTORE: &str = "\u{29c9}";
const GLYPH_CLOSE: &str = "\u{2715}";

/// Whether a left press that then moves should start a window drag. Held per
/// window so a plain click still reaches the double-click handler.
struct DragArm {
    armed: bool,
}

impl gpui::Render for DragArm {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The strip the user grabs to move the window.
///
/// Takes no children by design. A `Drag` hitbox is registered before its
/// descendants' and the lookup returns the first match, so a control nested
/// here would be shadowed unless it also called `.occlude()` — a step that
/// fails silently and cannot be reproduced on macOS. Keeping the strip empty
/// makes the mistake unrepresentable.
pub(crate) fn drag_region(tier: ControlTier, window: &mut Window, cx: &mut App) -> Stateful<Div> {
    let base = div().id("title-bar-drag").flex_1().h_full();
    match tier {
        // Windows answers WM_NCHITTEST from this hitbox and takes the drag,
        // double-click-to-maximize and Aero Shake with it.
        ControlTier::Hitbox => base.window_control_area(WindowControlArea::Drag),
        // Linux discards that callback in both backends, so the app asks the
        // compositor to move the window itself.
        ControlTier::Handlers => {
            let arm = window.use_state(cx, |_, _| DragArm { armed: false });
            base.on_mouse_down(
                MouseButton::Left,
                window.listener_for(&arm, |arm, _, _, _| arm.armed = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                window.listener_for(&arm, |arm, _, _, _| arm.armed = false),
            )
            .on_mouse_move(window.listener_for(&arm, |arm, _, window, _| {
                if arm.armed {
                    arm.armed = false;
                    window.start_window_move();
                }
            }))
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    window.zoom_window();
                }
            })
        }
    }
}

/// Minimize / maximize-or-restore / close, flush to the title bar's right edge.
pub(crate) fn window_controls(chrome: WindowChrome, window: &Window, cx: &App) -> Div {
    let maximized = window.is_maximized();
    let (zoom_glyph, zoom_id) = if maximized {
        (GLYPH_RESTORE, "window-restore")
    } else {
        (GLYPH_MAXIMIZE, "window-maximize")
    };

    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .h(px(theme::TITLE_BAR_HEIGHT))
        .child(control(chrome.tier, "window-minimize", GLYPH_MINIMIZE, cx))
        .child(control(chrome.tier, zoom_id, zoom_glyph, cx))
        .child(control(chrome.tier, "window-close", GLYPH_CLOSE, cx))
}

fn control(tier: ControlTier, id: &'static str, glyph: &'static str, cx: &App) -> Div {
    let button = crate::ui::button_window_control(id, glyph, id == "window-close", cx);
    let button = match tier {
        // The OS runs the action from the hit-test answer, and on Windows 11
        // hovering the maximize area is also what opens Snap Layouts.
        ControlTier::Hitbox => button,
        ControlTier::Handlers => button.on_click(move |_, window, _| match id {
            "window-minimize" => window.minimize_window(),
            "window-close" => window.remove_window(),
            _ => window.zoom_window(),
        }),
    };

    let area = match id {
        "window-minimize" => WindowControlArea::Min,
        "window-close" => WindowControlArea::Close,
        _ => WindowControlArea::Max,
    };
    div()
        .flex_none()
        // `.occlude()` keeps this hitbox out of the drag strip's shadow even
        // if the two ever end up nested; harmless while they are siblings.
        .occlude()
        .window_control_area(area)
        .child(button)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The glyph is the only thing telling the user which way the button goes,
    /// and it is the one control whose label changes with window state.
    #[test]
    fn maximize_and_restore_do_not_share_a_glyph() {
        assert_ne!(GLYPH_MAXIMIZE, GLYPH_RESTORE);
    }

    /// Every control carries a distinct glyph — a duplicate would read as two
    /// buttons doing the same thing.
    #[test]
    fn every_control_glyph_is_distinct() {
        let all = [GLYPH_MINIMIZE, GLYPH_MAXIMIZE, GLYPH_RESTORE, GLYPH_CLOSE];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "two controls share a glyph");
            }
        }
    }
}
