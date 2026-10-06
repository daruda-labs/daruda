//! Dock placeholder / empty-state message line.

use gpui::{Div, SharedString, div, prelude::*};

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
