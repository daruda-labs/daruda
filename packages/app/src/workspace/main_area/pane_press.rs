//! Pane activation for a press a scrollbar strip took.
//!
//! A strip stops its left press so content under it never reacts, which also
//! keeps the press from the wrapper's own "a left press activates this pane"
//! handler. This layer clears the strip's press record before dispatch and,
//! once dispatch is done, activates the pane if a strip took this press.
//! Anything else that stops a press (the Activity Bar) records nothing, so it
//! still activates nothing.

use gpui::{
    Context, HitboxBehavior, IntoElement, MouseButton, MouseDownEvent, Styled as _, canvas,
};

use super::pane_tree::PaneId;
use crate::ui::scrollbar::{forget_press, take_press};
use crate::workspace::Workspace;

/// An inset-0 layer to add as the pane wrapper's first child, so its hitbox
/// is behind everything the pane paints.
pub(super) fn blocked_press_layer(id: PaneId, cx: &mut Context<Workspace>) -> impl IntoElement {
    let workspace = cx.entity().downgrade();
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        // SAFETY-style exception to "canvas only caches geometry": the press
        // must be tested against this pane's hitbox, which only a canvas can
        // insert here, and the deferred work only dispatches pane activation.
        move |_, hitbox, window, _| {
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                // Not hovered when a popover over the pane occludes the press.
                if !phase.capture()
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                forget_press(cx);
                let (at, workspace) = (event.position, workspace.clone());
                window.defer(cx, move |window, cx| {
                    if take_press(at, cx) {
                        // SILENT-OK: a released Workspace has no pane left to activate.
                        let _ =
                            workspace.update(cx, |ws, cx| ws.focus_pane_on_click(id, window, cx));
                    }
                });
            });
        },
    )
    .absolute()
    .size_full()
}

#[cfg(test)]
mod tests {
    // Behaviour is covered end to end in `workspace::tests::pane_scrollbar_press`
    // and `workspace::tests::activity_bar_press`: both need real panes painted.
}
