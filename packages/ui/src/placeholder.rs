//! Dock placeholder / empty-state message line.

use crate::theme;
use gpui::{App, Div, SharedString, div, prelude::*, px};

/// Quiet text with explicit typography, foreground and wrapping width.
pub fn message_line(msg: impl Into<SharedString>, t: &theme::DarudaTheme) -> Div {
    div()
        .w_full()
        .min_w_0()
        .text_size(px(theme::FONT_SIZE_MD))
        .line_height(px(theme::INPUT_LINE_HEIGHT))
        .text_color(t.text_subtle)
        .child(msg.into())
}

pub fn empty_message(msg: impl Into<SharedString>, cx: &App) -> Div {
    message_line(msg, theme::current(cx))
}

/// A placeholder or empty-state message line for a dock view. Pins the
/// width (`w_full`) so a long sentence wraps onto multiple lines instead
/// of running off as one line and being clipped when the dock is narrow,
/// and centers each line (`text_center`). Font size and color are
/// inherited from the caller's container, so the same primitive fits any
/// dock's typography — fill/center chrome (e.g. `flex_1` + `items_center`)
/// is the caller's responsibility.
pub fn placeholder_text(msg: impl Into<SharedString>) -> Div {
    div().w_full().text_center().child(msg.into())
}
