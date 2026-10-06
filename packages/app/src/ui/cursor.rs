//! A cursor that outlives the drag it belongs to.
//!
//! gpui offers two ways to say what the pointer looks like, and which one is
//! right depends on whether a button is down. [`Window::set_cursor_style`]
//! binds the style to a hitbox and re-resolves it against the pointer every
//! frame; [`Window::set_window_cursor_style`] applies to the whole window and
//! is not hit-tested at all.
//!
//! A drag needs the second, for two reasons that both bite:
//!
//! The pointer leaves. Dragging a divider is dragging it *away* from the few
//! pixels it occupies, and a hitbox-bound style stops applying the moment it
//! does.
//!
//! And hitbox ids are not stable. `Window::next_hitbox_id` counts up for the
//! life of the window and is never reset, so an element that re-renders — which
//! anything being dragged does, every frame — is given a *new* id each time.
//! The style is resolved at the end of the draw against the hit test taken
//! from the frame before it, which knows only the old ids, so the lookup misses
//! and the pointer falls back to an arrow until the next mouse event puts it
//! right. At frame rate that reads as a flicker.
//!
//! zed answers this the same way in all four places it has the problem — the
//! pane divider (`workspace/pane_group.rs`), the editor's scrollbar and
//! minimap (`editor/element.rs`), and the scrollbar component
//! (`ui/components/scrollbar.rs`): hitbox-bound while idle, window-wide while
//! dragging. [`cursor_layer`] is that, as one element.

use gpui::{
    App, Bounds, CursorHideMode, CursorStyle, Global, IntoElement, ParentElement, Pixels, Styled,
    Window, canvas,
};

/// How far a cursor style reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorReach {
    /// Over the element it is attached to, and only while the pointer is
    /// there. What a resize handle wants when nobody is holding it.
    Hovered(CursorStyle),
    /// Everywhere, until it stops being asked for. What that same handle wants
    /// while it is being dragged, since the pointer is by then somewhere else.
    Dragging(CursorStyle),
}

impl CursorReach {
    /// `style`, reaching as far as `dragging` says it has to.
    ///
    /// The two arguments are folded here rather than at each call site so the
    /// rule is written once: every caller with a drag makes the same choice,
    /// and one that inlined it could quietly make the other one.
    pub fn while_dragging(style: CursorStyle, dragging: bool) -> Self {
        if dragging {
            Self::Dragging(style)
        } else {
            Self::Hovered(style)
        }
    }
}

/// Ask for the pointer's shape over an element, however far it has to reach.
pub trait CursorReachExt: Sized {
    /// `None` leaves the pointer to whatever is underneath.
    fn cursor_reach(self, reach: Option<CursorReach>) -> Self;
}

/// Blanket, so `div()` and the `Stateful` an `.id()` turns it into are both
/// covered — the three call sites are split across the two.
impl<T: Styled + ParentElement> CursorReachExt for T {
    fn cursor_reach(self, reach: Option<CursorReach>) -> Self {
        match reach {
            // The ordinary way, and deliberately the element's own hitbox
            // rather than one of our making: this is what a style on a `div`
            // has always been, and it is what zed keeps for the idle half of
            // the same choice.
            Some(CursorReach::Hovered(style)) => self.cursor(style),
            // A hitbox cannot answer for this one — see the module doc — so it
            // goes through a child that can reach paint.
            Some(CursorReach::Dragging(style)) => self.child(window_cursor(style)),
            None => self,
        }
    }
}

/// How many callers are keeping the pointer on screen, and what to put back
/// when the last of them lets go.
///
/// Counted rather than saved per caller: the policy is one app-wide value, and
/// two callers each saving "what it was" means the second saves the first's
/// override and hands it back as if it were the app's own. Two flow panes were
/// enough to leave hide-on-typing off for the rest of the session.
#[derive(Default)]
struct PointerHeldVisible {
    holders: usize,
    restore: Option<CursorHideMode>,
}

impl Global for PointerHeldVisible {}

/// Stop gpui hiding the pointer on keystrokes until [`release_pointer_visible`]
/// is called as many times as this was.
///
/// For a key that is *held* rather than typed — its auto-repeat otherwise
/// re-hides the pointer every tick, and the platform only brings it back when
/// the mouse moves, so holding it still leaves nothing on screen.
pub fn hold_pointer_visible(cx: &mut App) {
    let was = cx.cursor_hide_mode();
    let held = cx.default_global::<PointerHeldVisible>();
    held.holders += 1;
    if held.holders == 1 {
        held.restore = Some(was);
        cx.set_cursor_hide_mode(CursorHideMode::Never);
    }
}

/// Undo one [`hold_pointer_visible`]. The app's own policy comes back when the
/// last holder releases; releasing more times than it was held does nothing.
pub fn release_pointer_visible(cx: &mut App) {
    let held = cx.default_global::<PointerHeldVisible>();
    if held.holders == 0 {
        return;
    }
    held.holders -= 1;
    if held.holders > 0 {
        return;
    }
    if let Some(mode) = held.restore.take() {
        cx.set_cursor_hide_mode(mode);
    }
}

/// An element that paints nothing and asks for `style` across the whole window
/// for as long as it is painted.
fn window_cursor(style: CursorStyle) -> impl IntoElement {
    canvas(
        move |_bounds: Bounds<Pixels>, _: &mut Window, _: &mut App| {
            #[cfg(test)]
            painted::record(style, _bounds);
        },
        move |_, (), window: &mut Window, _: &mut App| window.set_window_cursor_style(style),
    )
    .absolute()
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule the three call sites share. Stated as a test because getting
    /// it backwards is invisible until someone drags something.
    #[test]
    fn a_drag_reaches_past_the_element_it_started_on() {
        assert_eq!(
            CursorReach::while_dragging(CursorStyle::ResizeLeftRight, true),
            CursorReach::Dragging(CursorStyle::ResizeLeftRight),
        );
        assert_eq!(
            CursorReach::while_dragging(CursorStyle::ResizeLeftRight, false),
            CursorReach::Hovered(CursorStyle::ResizeLeftRight),
        );
    }
}

/// What each window-wide cursor layer was asked for, and how big it came out.
/// A cursor cannot be read back from gpui, so this is how a test says the
/// layer reached the pointer at all.
#[cfg(test)]
pub mod painted {
    use gpui::{Bounds, CursorStyle, Pixels};
    use std::cell::RefCell;

    thread_local! {
        static LAST: RefCell<Vec<(CursorStyle, Bounds<Pixels>)>> =
            const { RefCell::new(Vec::new()) };
    }

    pub(super) fn record(style: CursorStyle, bounds: Bounds<Pixels>) {
        LAST.with(|last| last.borrow_mut().push((style, bounds)));
    }

    /// Every layer laid out since the last [`clear`].
    pub fn all() -> Vec<(CursorStyle, Bounds<Pixels>)> {
        LAST.with(|last| last.borrow().clone())
    }

    pub fn clear() {
        LAST.with(|last| last.borrow_mut().clear());
    }
}
