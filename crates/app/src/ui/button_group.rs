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

use gpui::{App, ElementId};
use gpui_component::Sizable as _;
use gpui_component::button::ButtonVariants as _;

use crate::ui::theme::PaneSurfaceTokens;

pub use gpui_component::button::ButtonGroup;

/// Single-select segmented button group at daruda's compact size, with
/// the selected segment filled in the accent colour.
pub fn button_group(id: impl Into<ElementId>) -> ButtonGroup {
    ButtonGroup::new(id).small().primary().outline()
}

/// Same strip for a pane-local surface (file viewer, agent chat) — segments
/// sit on the pane's own tint and mark the selection with its active tint.
///
/// [`button_group`]'s Primary-outline chrome is wrong twice over here: the
/// upstream outline path fills every segment with `theme().background`, i.e.
/// the app canvas rather than the pane's terminal-mirrored surface (each
/// segment reads as a black well), and it rings all of them in accent —
/// DESIGN.md rations accent to 3–4 visible elements and never as a fill.
/// Selected-segment text stays the caller's to set; the variant carries one
/// foreground for every state.
pub fn button_group_on_surface(
    id: impl Into<ElementId>,
    surface: &PaneSurfaceTokens,
    cx: &App,
) -> ButtonGroup {
    let variant =
        crate::ui::button::surface_button_variant(surface, cx).border(surface.border_tint);
    ButtonGroup::new(id).small().custom(variant)
}
