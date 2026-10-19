//! Tooltip helper — returns a closure GPUI's `.tooltip(...)` accepts.
//!
//! Wraps `gpui_component::tooltip::Tooltip` so callers don't import
//! the underlying type directly.
//!
//! Usage:
//! ```ignore
//! div().child("…").tooltip(crate::ui::tooltip::text("Open in Finder"))
//! ```

use gpui::{AnyView, App, FontWeight, SharedString, Window, div, prelude::*, px};
use gpui_component::tooltip::Tooltip;

use crate::theme;

pub fn text(
    content: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let content: SharedString = content.into();
    move |window, cx| Tooltip::new(content.clone()).build(window, cx)
}

pub fn with_description(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let title = title.into();
    let description = description.into();
    move |window, cx| {
        let title = title.clone();
        let description = description.clone();
        Tooltip::element(move |_, cx| {
            let theme = theme::current(cx);
            div()
                .flex()
                .flex_col()
                .gap(px(theme::GAP_XS))
                .child(
                    div()
                        .text_size(px(theme::FONT_SIZE_MD))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text_primary)
                        .child(title.clone()),
                )
                .child(
                    div()
                        .text_size(px(theme::FONT_SIZE_SM))
                        .text_color(theme.text_muted)
                        .child(description.clone()),
                )
        })
        .build(window, cx)
    }
}
