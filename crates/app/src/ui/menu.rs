use gpui::{Context, Window};

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

#[cfg(test)]
mod tests {}
