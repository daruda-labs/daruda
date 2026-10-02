use std::rc::Rc;

use gpui::{
    App, Context, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    Point, Styled, Window, anchored, deferred, div, px,
};

// `ContextMenuExt` is deliberately NOT re-exported. Its `.context_menu(..)`
// renders the menu inside the caller's own subtree, where an ancestor's clip
// cuts it — visually and for hit-testing. Right-click menus go through
// `crate::workspace::root_menu::RootContextMenuExt`; leaving the trait
// unexported makes the broken form a compile error rather than a convention,
// since `scripts/lint-direct-gpui-component.sh` already blocks importing it
// straight from the vendored crate.
pub use gpui_component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};

/// Wrap a menu builder closure with the daruda compact-size default
/// (`PopupMenu::small()`), so call sites never manage sizing manually.
///
/// Needed at `.dropdown_menu(...)` call sites; right-click menus get it for
/// free because `RootContextMenuExt::root_context_menu` applies it.
///
/// ```ignore
/// .dropdown_menu(crate::ui::menu_builder(move |menu, _, _| {
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

/// [`menu_builder`] for a menu opened from inside another float — a popover
/// panel's own dropdown. Both would otherwise share one surface and a 1.06:1
/// hairline, so the menu vanished into the panel behind it. It takes the next
/// lift above the float and a `text_subtle` edge (>= 3:1 in both themes).
pub fn stacked_menu_builder<F>(
    f: F,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static
where
    F: Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
{
    move |menu, window, cx| f(menu.small().chrome(stacked_menu_chrome), window, cx)
}

/// The stacked menu's fill and edge, from the active theme.
fn stacked_menu_chrome(cx: &App) -> (gpui::Hsla, gpui::Hsla) {
    stacked_menu_chrome_of(super::theme::current(cx))
}

fn stacked_menu_chrome_of(t: &super::theme::DarudaTheme) -> (gpui::Hsla, gpui::Hsla) {
    (t.float_panel_bg.blend(t.overlay_active), t.text_subtle)
}

/// Render a `PopupMenu` at a fixed window position — the one way a
/// right-click menu is painted, from the workspace root so no ancestor clip
/// can reach it (see `Workspace::open_context_menu` and
/// `crate::workspace::root_menu`). Reproduces the same
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
mod tests {
    use super::stacked_menu_chrome_of;
    use crate::ui::theme::{DarudaTheme, contrast_ratio};

    /// In both shipped themes the stacked menu leaves the float rung, and its
    /// edge holds the 3:1 a control boundary needs against the panel behind.
    #[test]
    fn a_stacked_menu_stands_off_the_float_it_opens_over() {
        let light: DarudaTheme =
            serde_json::from_str(include_str!("../../../../assets/themes/daruda_light.json"))
                .unwrap();
        for theme in [DarudaTheme::default(), light] {
            let (bg, border) = stacked_menu_chrome_of(&theme);
            assert_ne!(bg, theme.float_panel_bg);
            assert!(contrast_ratio(border, theme.float_panel_bg) >= 3.0);
            assert!(contrast_ratio(border, bg) >= 3.0);
        }
    }
}
