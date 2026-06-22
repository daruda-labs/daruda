//! Segmented single-select button strip — `small()` + accent-outline.
//!
//! A theme-aware replacement for hand-rolled chip strips. Styled as a
//! Primary **outline** group so the selected segment fills with the
//! accent (`primary_active`) and white text while the unselected ones
//! stay outlined — the selected state reads clearly in both light and
//! dark. (The default Secondary variant routes "selected" through
//! `secondary_active`, which is the press-recess surface — the lightest
//! canvas in light mode — so the selected chip washes out to near-white.)
//!
//! Single-select (`multiple == false`); `.on_click(|indices, _, _|)`
//! reports the selected child indices and the caller maps the index
//! back to its domain value.

use gpui::ElementId;
use gpui_component::Sizable as _;
use gpui_component::button::ButtonVariants as _;

pub use gpui_component::button::ButtonGroup;

/// Single-select segmented button group at daruda's compact size, with
/// the selected segment filled in the accent colour.
pub fn button_group(id: impl Into<ElementId>) -> ButtonGroup {
    ButtonGroup::new(id).small().primary().outline()
}
