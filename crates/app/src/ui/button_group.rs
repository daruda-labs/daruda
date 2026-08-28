//! Segmented single-select button strip — `small()`, hairline-outlined, with
//! the selected segment filled in the accent.
//!
//! Selection is the one thing on the strip that gets accent: a background lift
//! alone is too subtle for a choice control (DESIGN.md §Do's), and the fill is
//! genuine state rather than decoration. Everything else stays neutral —
//! resting labels take the muted text tone and the frame takes the hairline.
//!
//! Single-select (`multiple == false`); `.on_click(|indices, _, _|)`
//! reports the selected child indices and the caller maps the index
//! back to its domain value.

use gpui::{App, ElementId};
use gpui_component::Sizable as _;
use gpui_component::button::{ButtonCustomVariant, ButtonVariants as _};

use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;

pub use gpui_component::button::ButtonGroup;

/// Single-select segmented button group at daruda's compact size, for app
/// chrome (modals, popovers) that follows the UI theme.
///
/// Not the upstream Primary-**outline** chrome, which paints every *unselected*
/// segment's label and border in `accent`. Two DESIGN.md rules land on that at
/// once: accent as small text misses the readability band (3.89:1 on the
/// popover surface this strip actually sits on — `t.popover`, i.e. `surface-2`
/// — against a 4.5:1 floor), and the accent budget is 3–4 visible elements,
/// while the fold-rule editor puts twelve strips on screen at once.
///
/// Tones are picked against their own backgrounds: `text_muted` on the popover
/// (5.65:1) and `accent_fg` on the accent fill (4.70:1). `text_primary` on the
/// fill measures 4.40:1 and would sit under the floor, which is why the selected
/// tone is a separate slot rather than one shared foreground.
///
/// The frame stays on the global `hairline` (1.19:1) rather than the 3:1 edge
/// an Activity Bar chip carries. That floor applies where the edge is the
/// *only* thing separating a control from adjacent non-interactive text; here
/// the ≥4.5:1 label and the accent-filled selected segment identify both the
/// control and its state, so the frame is refinement.
pub fn button_group(id: impl Into<ElementId>, cx: &App) -> ButtonGroup {
    let t = theme::current(cx);
    let variant = ButtonCustomVariant::new(cx)
        .foreground(t.text_muted)
        .selected_foreground(theme::ACCENT_FG)
        .border(t.border)
        .hover(t.button_widget_bg_hover)
        .active(theme::PRIMARY);
    ButtonGroup::new(id).small().custom(variant)
}

/// Same strip for a pane-local surface (file viewer, agent chat) — segments
/// sit on the pane's own tint and mark the selection with its active tint.
///
/// The pane mirrors the *terminal* palette, which the UI theme's accent knows
/// nothing about, so this one cannot borrow [`button_group`]'s fill: the pair
/// would be an unverified contrast on whatever background the user's terminal
/// preset supplies. Colours come from the surface instead, and the selected
/// segment brightens its label to the surface's full foreground so the fill is
/// not carrying the state on its own.
pub fn button_group_on_surface(
    id: impl Into<ElementId>,
    surface: &PaneSurfaceTokens,
    cx: &App,
) -> ButtonGroup {
    let variant = crate::ui::button::surface_button_variant(surface, cx)
        .selected_foreground(surface.foreground)
        .border(surface.border_tint);
    ButtonGroup::new(id).small().custom(variant)
}
