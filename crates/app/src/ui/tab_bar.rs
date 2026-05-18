//! `tab_bar` / `tab` factories — `Small + underline` baked in, with
//! `text_xs` cascaded from the bar so labels stay compact even at the
//! taller Small strip height.
//!
//! Call surface:
//! `tab_bar(id).w_full().selected_index(ix).children([tab(a), tab(b), ...]).on_click(...)`
//!
//! `tab(label)` injects horizontal padding so the underline border and
//! click target widen with the label. `TabVariant::Underline` zeros the
//! upstream `inner_paddings` (`gpui_component/src/tab/tab.rs:106`), so
//! without this baked padding the underline sits flush against the
//! label glyphs.
//!
//! Font-size independence from `Size` relies on a vendor patch that
//! drops the size→text_xs/text_sm/text_base map inside `Tab::render`;
//! see `crates/app/src/ui/CLAUDE.md` (vendor patches table).
//!
//! `prefix` / `menu` are intentionally out of scope. `suffix` is
//! reserved for the bottom dock's `+` action chip (panel-tab creation);
//! every other tab strip (left/right dock) keeps action chips in
//! the panel body's section header.

use gpui::{ElementId, SharedString, Styled as _};
use gpui_component::{Sizable as _, Size};

pub use gpui_component::tab::{Tab, TabBar};

/// Underline-style `Small` TabBar with `text_xs` cascaded to children.
pub fn tab_bar(id: impl Into<ElementId>) -> TabBar {
    TabBar::new(id).with_size(Size::Small).underline().text_xs()
}

/// Tab paired with [`tab_bar`]. Adds 10px horizontal padding on the base
/// so the underline border + click target widen past the bare label.
/// Variant and size cascade from the parent `TabBar`, so we don't bake
/// `.underline()` / `.with_size(...)` here.
pub fn tab(label: impl Into<SharedString>) -> Tab {
    Tab::new().label(label).px_2p5()
}
