use std::rc::Rc;

use gpui::{
    App, Context, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    Point, Styled, Window, anchored, deferred, div, px,
};

pub use gpui_component::menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem};

/// Wrap a menu builder closure with the daruda compact-size default
/// (`PopupMenu::small()`). Use for both `.context_menu(...)` and
/// `.dropdown_menu(...)` so call sites never manage sizing manually.
///
/// ```ignore
/// .context_menu(crate::ui::menu_builder(move |menu, _, _| {
///     menu.item(PopupMenuItem::new("Action").on_click(...))
/// }))
/// ```
pub fn menu_builder<F>(
    f: F,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static
where
    F: Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
{
    move |menu, window, cx| f(menu.small(), window, cx)
}

/// Render an imperatively-opened `PopupMenu` (System B) at a fixed
/// window position — the counterpart to `.context_menu(...)` for call
/// sites that build the menu at click time rather than declaratively
/// (see `Workspace::open_context_menu`). Reproduces the same
/// full-window occluding-backdrop + anchored-menu shape as upstream's
/// own declarative `ContextMenu<E>` element
/// (`gpui_component::menu::context_menu`), so outside clicks are
/// blocked from reaching whatever sits underneath and route back to
/// `on_dismiss`. `crate::ui` must stay domain-agnostic, so the caller
/// (which owns the `Workspace` context) supplies the close callback.
pub fn popup_menu_deferred(
    menu: &Entity<PopupMenu>,
    position: Point<Pixels>,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    // Shared so both button handlers can move their own clone — `on_dismiss`
    // is an opaque `impl Fn`, not guaranteed `Copy`.
    let on_dismiss = Rc::new(on_dismiss);
    let on_dismiss_right = on_dismiss.clone();
    deferred(
        anchored().child(
            div()
                .size_full()
                .occlude()
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    on_dismiss(window, cx)
                })
                .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                    on_dismiss_right(window, cx)
                })
                .child(
                    anchored()
                        .position(position)
                        .snap_to_window_with_margin(px(
                            crate::ui::theme::POPUP_MENU_DEPLOY_EDGE_MARGIN,
                        ))
                        .child(menu.clone()),
                ),
        ),
    )
    .with_priority(1)
}

#[cfg(test)]
mod tests {}
