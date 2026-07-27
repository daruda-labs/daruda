//! Button factories — `small()` auto-applied + cycled out of Tab
//! navigation by default.
//!
//! daruda treats keyboard Tab as "next input field" rather than the
//! full focus tree, so footer buttons (Cancel / Save / Delete) sit
//! outside the cycle — otherwise pressing Tab from the last input
//! lands on Cancel before wrapping to the first input, which is the
//! wrong mental model for a form. Users still reach Cancel / Save
//! via Escape / Enter (Dialog provides both), or by clicking. Callers
//! that *want* a button inside the cycle chain
//! `.tab_stop(true).tab_index(n)` explicitly.

use crate::ui::theme;
use gpui::{App, ElementId, SharedString, Styled as _, px};
use gpui_component::Sizable as _;
use gpui_component::button::{ButtonCustomVariant, ButtonVariants as _};

pub use gpui_component::button::Button;

/// Default secondary button — `small()` + label, excluded from Tab.
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id).small().label(label).tab_stop(false)
}

/// Primary variant.
pub fn button_primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    button(id, label).primary()
}

/// Danger variant.
pub fn button_danger(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    button(id, label).danger()
}

/// Bare button (no label) — for icon-only buttons where the caller
/// chains `.icon(...)`.
pub fn button_bare(id: impl Into<ElementId>) -> Button {
    Button::new(id).small().tab_stop(false)
}

/// Chip-style button — outlined, compact padding, `xsmall` text,
/// forced to a uniform `BUTTON_CHIP_SIZE` square. Use when two small
/// glyph-only buttons sit adjacent (e.g. the bottom dock tab strip's
/// `+` and row-preset chips) and need to read as discrete equal-weight
/// controls rather than a run-on glyph sequence.
pub fn button_chip(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    button(id, label)
        .outline()
        .compact()
        .w(px(theme::BUTTON_CHIP_SIZE))
        .h(px(theme::BUTTON_CHIP_SIZE))
}

/// Always-visible `×` close glyph — muted at rest, fills bg with the
/// destructive `ERROR` token on direct hover. The single close affordance
/// shared by the tab cell, pane header, and task_edit rows: all three show
/// the `×` unconditionally (no hover gating). `xsmall` + the compact
/// pane-header box keeps it fitting the short pane-header height.
pub fn button_close(id: impl Into<ElementId>, cx: &App) -> Button {
    let t = theme::current(cx);
    let variant = ButtonCustomVariant::new(cx)
        .foreground(t.text_muted)
        .hover(theme::ERROR);
    Button::new(id)
        .xsmall()
        .tab_stop(false)
        .custom(variant)
        .label("\u{00d7}")
        .w(px(theme::PANE_HEADER_CLOSE_W))
        .h(px(theme::PANE_HEADER_CLOSE_H))
        .p(px(0.))
        .rounded(px(theme::PANE_HEADER_CLOSE_RADIUS))
        .text_size(px(theme::PANE_HEADER_CLOSE_FONT_SIZE))
}

/// Destructive `×` glyph for hover-revealed row actions — muted at
/// rest, fills with the `ERROR` token on direct hover so the
/// destructive action reads clearly only when the pointer is on it.
/// Unlike [`button_close`] it bakes no visibility gating or fixed
/// pane-header sizing: the caller's own hover-reveal container
/// controls when it appears, and `xsmall` keeps it compact next to
/// other row-action chips.
pub fn button_delete_glyph(id: impl Into<ElementId>, cx: &App) -> Button {
    let t = theme::current(cx);
    let variant = ButtonCustomVariant::new(cx)
        .foreground(t.text_muted)
        .hover(theme::ERROR);
    Button::new(id)
        .xsmall()
        .tab_stop(false)
        .custom(variant)
        .label("\u{00d7}")
}

/// `✎` edit glyph for hover-revealed row actions — muted at rest, brightens to
/// the primary text tone on direct hover. Sibling of [`button_delete_glyph`]
/// (same compact `xsmall`, no fixed sizing) for the queued-prompt strip's
/// per-item edit affordance.
pub fn button_edit_glyph(id: impl Into<ElementId>, cx: &App) -> Button {
    let t = theme::current(cx);
    let variant = ButtonCustomVariant::new(cx)
        .foreground(t.text_muted)
        .hover(t.text_primary);
    Button::new(id)
        .xsmall()
        .tab_stop(false)
        .custom(variant)
        .label("\u{270e}")
}

/// `↩` cancel-edit glyph — muted at rest, brightens to the primary text tone on
/// direct hover. Shown on the queued-prompt row currently being edited, in
/// place of the edit / delete glyphs.
pub fn button_edit_cancel_glyph(id: impl Into<ElementId>, cx: &App) -> Button {
    let t = theme::current(cx);
    let variant = ButtonCustomVariant::new(cx)
        .foreground(t.text_muted)
        .hover(t.text_primary);
    Button::new(id)
        .xsmall()
        .tab_stop(false)
        .custom(variant)
        .label("\u{21a9}")
}

/// Section-header action glyph (`+`, `⟳`, `▾`, ...) — muted text on
/// transparent bg with a soft hover-fill, no border. Use for inline
/// header affordances and small dismiss buttons.
pub fn button_header_action(
    id: impl Into<ElementId>,
    icon: impl Into<SharedString>,
    cx: &App,
) -> Button {
    let t = theme::current(cx);
    let variant = ButtonCustomVariant::new(cx)
        .foreground(t.text_muted)
        .hover(t.text_primary);
    Button::new(id)
        .small()
        .tab_stop(false)
        .custom(variant)
        .label(icon)
}

/// Status-bar account slot — the dropdown trigger showing the focused
/// pane's account (or "System"). `ghost()` paints no border in any state
/// (see `ButtonVariant::Ghost::border_color`), which left it reading as
/// plain text next to the status bar's other muted labels; this bakes an
/// always-on hairline border plus a fixed compact height so it reads as a
/// clickable control at rest, not just on hover.
pub fn button_status_account(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Button {
    let t = theme::current(cx);
    let variant = ButtonCustomVariant::new(cx)
        .foreground(t.text_muted)
        .border(t.border)
        .hover(t.status_bar_account_hover_bg);
    Button::new(id)
        .xsmall()
        .tab_stop(false)
        .custom(variant)
        .label(label)
        .h(px(theme::STATUS_BAR_ACCOUNT_HEIGHT))
        .px(px(theme::STATUS_BAR_ACCOUNT_PAD_X))
        .rounded(px(theme::STATUS_BAR_ACCOUNT_RADIUS))
}

/// `+` tile sized to align with [`crate::ui::MacroKey`] icon cells in
/// the bottom-dock grid — square footprint with a dashed outline that
/// fills bg on hover.
pub fn button_add_tile(id: impl Into<ElementId>, cx: &App) -> Button {
    let t = theme::current(cx);
    let variant = ButtonCustomVariant::new(cx)
        .foreground(t.text_muted)
        .border(t.text_muted)
        .hover(t.button_widget_bg_hover);
    Button::new(id)
        .small()
        .tab_stop(false)
        .custom(variant)
        .label("+")
        .w(px(theme::BUTTON_WIDGET_HEIGHT))
        .h(px(theme::BUTTON_WIDGET_HEIGHT))
        .p(px(0.))
        .rounded(px(theme::BUTTON_WIDGET_RADIUS))
        .border(px(theme::BUTTON_WIDGET_ADD_BORDER_W))
        .border_dashed()
        .text_size(px(theme::BUTTON_WIDGET_FONT_SIZE))
}

/// Dock toggle (◨ ⊞ ◧). `active=true` → filled bg + bright icon;
/// `active=false` → subdued, with a hover-bg that previews the
/// active fill.
pub fn button_toggle(
    id: impl Into<ElementId>,
    icon: impl Into<SharedString>,
    active: bool,
    cx: &App,
) -> Button {
    let t = theme::current(cx);
    let fg = if active { t.text_primary } else { t.text_muted };
    let active_bg = t.dock_icon_active_bg;
    let variant = if active {
        ButtonCustomVariant::new(cx)
            .color(active_bg)
            .foreground(fg)
            .hover(active_bg)
    } else {
        ButtonCustomVariant::new(cx).foreground(fg).hover(active_bg)
    };
    Button::new(id)
        .small()
        .tab_stop(false)
        .custom(variant)
        .label(icon)
        .w(px(theme::DOCK_ICON_BUTTON_W))
        .h(px(theme::DOCK_ICON_BUTTON_H))
        .p(px(0.))
        .rounded(px(theme::DOCK_ICON_BUTTON_RADIUS))
        .text_size(px(theme::DOCK_ICON_SIZE))
}
