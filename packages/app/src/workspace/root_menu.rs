//! Right-click menus that deploy at the workspace root.
//!
//! The vendored `.context_menu(...)` renders its menu with `deferred()` from
//! *inside* the attaching element's subtree. `deferred` does not escape an
//! ancestor's clip: gpui captures the ambient `content_mask` into
//! `DeferredDraw` and re-applies it when the deferred draw paints, and
//! `Window::with_content_mask` only ever *intersects*, so nothing inside the
//! menu can widen it. `Frame::hit_test` then intersects each hitbox with that
//! same mask — so a menu opened from a row inside a scroll container is not
//! just visually cut at the container's edge, the overflowing part is
//! unclickable.
//!
//! Every daruda surface that carries a row-level menu is inside one: the dock
//! bodies clip by definition (`left_dock::left_panel_body`), lists scroll, and
//! panes clip their content. So this is the path every app call site takes —
//! [`Workspace::open_context_menu`] deploys at the workspace root, outside
//! every clip, which is what the pane menu has always done.
//!
//! **Scope.** This covers right-click menus only. `.dropdown_menu(...)` routes
//! through `Popover`, which builds the same `deferred(anchored(..))` from the
//! trigger's own subtree and so carries the identical defect; its menus are
//! small and open next to their trigger, so the clip bites less often, but it
//! is unfixed. Do not read this module as "the clipping problem is gone".
//!
//! **One behaviour is traded away.** `popup_menu_deferred`'s backdrop
//! `occlude()`s, and an element listener is hitbox-gated, so right-clicking a
//! second row while a menu is open dismisses without reopening at the new spot
//! — two presses where the vendored path took one (its window-level listener
//! fired straight through the occluder). Deliberate: it matches the pane, tab
//! strip and terminal menus, and the alternative is dropping the occluder,
//! which also un-blocks hover, scroll and drag under an open menu.

use std::rc::Rc;

use gpui::{Context, InteractiveElement, MouseButton, WeakEntity, Window};

use crate::ui::{PopupMenu, menu_builder};
use crate::workspace::Workspace;

pub(in crate::workspace) trait RootContextMenuExt:
    InteractiveElement + Sized
{
    /// Attach a right-click menu that opens at the workspace root.
    ///
    /// Drop-in for the vendored `.context_menu(...)`, minus its two problems:
    /// the menu is not clipped by the caller's container, and — because this is
    /// an element listener that consumes the press rather than a window-level
    /// bounds test — an element nested inside another element's menu handler
    /// opens one menu, not both. The file-viewer path label inside the pane
    /// body is the case that needs it.
    ///
    /// Unlike `.context_menu(...)` it also returns `Self`, so it composes
    /// anywhere in the chain instead of having to come last.
    fn root_context_menu<F>(self, workspace: WeakEntity<Workspace>, build: F) -> Self
    where
        F: Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    {
        // Each press builds a fresh menu, so the builder outlives the call.
        let build = Rc::new(menu_builder(build));
        self.on_mouse_down(MouseButton::Right, move |event, window, cx| {
            // The element owns this press whether or not a menu follows, so
            // consume it before anything can bail — `MacroKey` does the same.
            cx.stop_propagation();
            let Some(workspace) = workspace.upgrade() else {
                return;
            };
            let position = event.position;
            let build = build.clone();
            // Build before leasing, never inside `workspace.update`: builders
            // read the workspace to decide their items (the file viewer reads
            // `active_lanes`), and `PopupMenu::build` runs the closure
            // synchronously — inside a lease that is a double-lease panic
            // (CLAUDE.md Pitfall 5). Same order as `tab_strip`'s deploy.
            let menu =
                PopupMenu::build(window, cx, move |menu, window, cx| build(menu, window, cx));
            workspace.update(cx, |workspace, cx| {
                workspace.open_context_menu(position, menu, window, cx);
            });
        })
    }
}

impl<E: InteractiveElement + Sized> RootContextMenuExt for E {}

#[cfg(test)]
mod tests {
    // The invariant this module exists for — a dock row's right-click deploys
    // at the workspace root rather than in its own subtree — needs a real
    // workspace and a synthetic press, so it lives with the other deploy tests
    // in `crate::workspace::tests::context_menu_ops`.
}
