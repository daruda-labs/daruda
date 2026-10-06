//! Agent mark — the embedded brand SVG, or a generic bot glyph for an agent
//! the catalog doesn't know.
//!
//! Paths come from [`crate::agent::icons`]. The SVGs are `fill="currentColor"`
//! monochrome, so `color` fully determines their appearance.

use gpui::{AnyElement, Hsla, IntoElement, Pixels, prelude::*, svg};

use crate::ui::{Icon, IconName, Sizable as _};

/// `path` is [`crate::agent::icons::icon_for_agent`]'s result; `None` falls
/// back to the bot glyph so an unknown agent still gets a mark. The fallback
/// carries its own `xsmall` metric — gpui_component's `Icon` sizes by tier,
/// not pixels — so `size` applies to the SVG branch only.
pub fn agent_icon(path: Option<&'static str>, size: Pixels, color: Hsla) -> AnyElement {
    match path {
        Some(path) => svg()
            .flex_none()
            .w(size)
            .h(size)
            .path(path)
            .text_color(color)
            .into_any_element(),
        None => Icon::new(IconName::Bot)
            .xsmall()
            .text_color(color)
            .into_any_element(),
    }
}

/// The same mark as a `gpui_component` [`Icon`], for slots that take one
/// instead of an element — `PopupMenuItem::icon`, notably. Falls back to the
/// bot glyph on `None`, matching [`agent_icon`].
pub fn agent_menu_icon(path: Option<&'static str>) -> Icon {
    match path {
        Some(path) => Icon::empty().path(path),
        None => Icon::new(IconName::Bot),
    }
}
