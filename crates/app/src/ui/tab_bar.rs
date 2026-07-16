//! `tab_bar` / `tab` factories with `Small + underline` baked in.
//!
//! `tab(label)` adds padding because upstream underline tabs zero their inner
//! padding. Font-size independence relies on the vendor patch documented in
//! `crates/app/src/ui/CLAUDE.md`; action chips stay outside this wrapper except
//! the bottom-dock suffix.

use gpui::{ElementId, SharedString, Styled as _};
use gpui_component::{Sizable as _, Size};

pub use gpui_component::tab::{Tab, TabBar};

/// Underline-style `Small` TabBar with `text_xs` cascaded to children.
pub fn tab_bar(id: impl Into<ElementId>) -> TabBar {
    TabBar::new(id).with_size(Size::Small).underline().text_xs()
}

/// Tab paired with [`tab_bar`]; padding widens the underline/click target.
pub fn tab(label: impl Into<SharedString>) -> Tab {
    Tab::new().label(label).px_2p5()
}
