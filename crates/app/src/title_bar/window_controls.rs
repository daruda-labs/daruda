//! The drag strip and the minimize / maximize / close controls the app draws
//! when the platform left it no caption.
//!
//! Reached only under [`TitleBarChrome::Client`](super::policy::TitleBarChrome),
//! so `ControlTier::Handlers` here means Linux: Windows resolves to `Hitbox`
//! and macOS never reaches the client arm at all.

use gpui::{
    App, Div, MouseButton, MouseMoveEvent, Stateful, Window, WindowControlArea, div, prelude::*, px,
};

use super::policy::{ControlTier, WindowChrome};
use crate::ui::theme;
use crate::workspace::CloseWindow;

/// Box-drawing glyphs rather than SVG: the dock toggles beside these already
/// read as glyphs, and the three shapes are unambiguous at 11px.
const GLYPH_MINIMIZE: &str = "\u{2500}";
const GLYPH_MAXIMIZE: &str = "\u{25a1}";
const GLYPH_RESTORE: &str = "\u{29c9}";
const GLYPH_CLOSE: &str = "\u{2715}";

/// One app-drawn caption control. An enum rather than a string id because the
/// area, the action and the danger tone all have to agree, and a `_` arm over
/// string literals would mis-wire a button with no compile error.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Control {
    Minimize,
    /// Maximize or restore, whichever the window is not.
    Zoom,
    Close,
}

impl Control {
    fn id(self) -> &'static str {
        match self {
            Self::Minimize => "window-minimize",
            Self::Zoom => "window-zoom",
            Self::Close => "window-close",
        }
    }

    fn glyph(self, maximized: bool) -> &'static str {
        match self {
            Self::Minimize => GLYPH_MINIMIZE,
            Self::Zoom if maximized => GLYPH_RESTORE,
            Self::Zoom => GLYPH_MAXIMIZE,
            Self::Close => GLYPH_CLOSE,
        }
    }

    fn area(self) -> WindowControlArea {
        match self {
            Self::Minimize => WindowControlArea::Min,
            Self::Zoom => WindowControlArea::Max,
            Self::Close => WindowControlArea::Close,
        }
    }

    fn is_close(self) -> bool {
        self == Self::Close
    }

    /// Whether the compositor will honour this control at all. Wayland
    /// advertises its capabilities per surface, so a compositor that refuses
    /// minimize would otherwise get a button that does nothing.
    fn supported(self, window: &Window) -> bool {
        let caps = window.window_controls();
        match self {
            Self::Minimize => caps.minimize,
            Self::Zoom => caps.maximize,
            Self::Close => true,
        }
    }
}

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
            // Keyed rather than caller-located: the id is what keeps two drag
            // strips in one window from sharing a flag.
            let arm: gpui::Entity<DragArm> =
                window.use_keyed_state(gpui::ElementId::from("title-bar-drag-arm"), cx, |_, _| {
                    DragArm { armed: false }
                });
            base.on_mouse_down(
                MouseButton::Left,
                window.listener_for(&arm, |arm, _, _, _| arm.armed = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                window.listener_for(&arm, |arm, _, _, _| arm.armed = false),
            )
            // Both `on_mouse_up` and `on_mouse_move` are hover-gated, so a
            // press that leaves the strip before releasing never clears the
            // flag. Re-checking the held button is what stops the window
            // following a later hover with no button down at all.
            .on_mouse_move(
                window.listener_for(&arm, |arm, ev: &MouseMoveEvent, window, _| {
                    let dragging = arm.armed && ev.pressed_button == Some(MouseButton::Left);
                    arm.armed = false;
                    if dragging {
                        window.start_window_move();
                    }
                }),
            )
            .on_mouse_down_out(window.listener_for(&arm, |arm, _, _, _| arm.armed = false))
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    window.zoom_window();
                }
            })
        }
    }
}

/// Minimize / maximize-or-restore / close, flush to the title bar's right edge.
//
// TODO(RK4): fullscreen still paints these; they should hide or shift when
// `window.is_fullscreen()`, which the plan left unresolved.
pub(crate) fn window_controls(chrome: WindowChrome, window: &Window, cx: &App) -> Div {
    let maximized = window.is_maximized();
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .h(px(theme::TITLE_BAR_HEIGHT))
        .children(
            [Control::Minimize, Control::Zoom, Control::Close]
                .into_iter()
                .filter(|c| c.supported(window))
                .map(|c| control(c, chrome.tier, maximized, cx)),
        )
}

fn control(control: Control, tier: ControlTier, maximized: bool, cx: &App) -> Div {
    let glyph = control.glyph(maximized);
    let button = if control.is_close() {
        crate::ui::button_window_control_danger(control.id(), glyph, cx)
    } else {
        crate::ui::button_window_control(control.id(), glyph, cx)
    };
    let button = match tier {
        // The OS runs the action from the hit-test answer, and on Windows 11
        // hovering the maximize area is also what opens Snap Layouts.
        ControlTier::Hitbox => button,
        ControlTier::Handlers => button.on_click(move |_, window, cx| match control {
            Control::Minimize => window.minimize_window(),
            Control::Zoom => window.zoom_window(),
            // Not `remove_window`: that skips the platform should-close
            // callback, and with it the prompt that holds a dirty task-edit
            // draft. The action routes back through the host's close flow.
            Control::Close => window.dispatch_action(Box::new(CloseWindow), cx),
        }),
    };

    div()
        .flex_none()
        // Keeps this hitbox out of a drag strip's shadow even if the two ever
        // end up nested; harmless while they are siblings.
        .occlude()
        .window_control_area(control.area())
        .child(button)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The glyph is the only thing telling the user which way the button goes,
    /// and Zoom is the one control whose label changes with window state.
    #[test]
    fn zoom_swaps_its_glyph_with_the_window_state() {
        assert_eq!(Control::Zoom.glyph(false), GLYPH_MAXIMIZE);
        assert_eq!(Control::Zoom.glyph(true), GLYPH_RESTORE);
    }

    /// A duplicate glyph or id would read as two buttons doing the same thing.
    #[test]
    fn every_control_is_distinct() {
        let all = [Control::Minimize, Control::Zoom, Control::Close];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.id(), b.id(), "two controls share an id");
                assert_ne!(a.glyph(false), b.glyph(false), "two controls share a glyph");
                assert_ne!(a.area(), b.area(), "two controls claim the same hit area");
            }
        }
    }

    /// Close is the one control the danger tone belongs to.
    #[test]
    fn only_close_is_destructive() {
        assert!(Control::Close.is_close());
        assert!(!Control::Minimize.is_close());
        assert!(!Control::Zoom.is_close());
    }
}
