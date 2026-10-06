//! Tooltip helper — returns a closure GPUI's `.tooltip(...)` accepts.
//!
//! Wraps `gpui_component::tooltip::Tooltip` so callers don't import
//! the underlying type directly.
//!
//! Usage:
//! ```ignore
//! div().child("…").tooltip(crate::ui::tooltip::text("Open in Finder"))
//! ```

use gpui::{AnyView, App, SharedString, Window};
use gpui_component::tooltip::Tooltip;

pub fn text(
    content: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let content: SharedString = content.into();
    move |window, cx| Tooltip::new(content.clone()).build(window, cx)
}
