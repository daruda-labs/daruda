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
use crate::ui::theme::PaneSurfaceTokens;
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

/// The one mapping from a pane-local surface's tokens to a button's colours.
/// Shared by the factories below and
/// [`button_group_on_surface`](crate::ui::button_group_on_surface), so a
/// standalone control and a segment of a strip on the same surface cannot
/// drift apart.
pub(crate) fn surface_button_variant(surface: &PaneSurfaceTokens, cx: &App) -> ButtonCustomVariant {
    ButtonCustomVariant::new(cx)
        .foreground(surface.foreground_muted)
        .hover(surface.tint)
        .active(surface.active_tint)
}

/// Labelled button for a pane-local surface — the agent-chat activity bar's
/// chips, and anything else sitting on a terminal-mirrored surface.
///
/// [`ghost`](gpui_component::button::ButtonVariants::ghost) resolves its
/// foreground from the *UI* theme (`secondary_foreground`), which has no
/// relationship to a surface that mirrors the *terminal* palette: `ui_preset`
/// and `terminal_preset` are independent config keys, so a light UI over a dark
/// pane leaves such a button at roughly 1.1:1 against the bar it sits on.
/// Colours come from the pane's own tokens instead — including the inactive-pane
/// dim, which the caller applies by handing over
/// [`PaneSurfaceTokens::dimmed`] tokens. Selection reads through the surface's
/// active tint, the same axis `ghost` used.
pub fn button_on_surface(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    surface: &PaneSurfaceTokens,
    cx: &App,
) -> Button {
    button(id, label).custom(surface_button_variant(surface, cx))
}

/// A labelled chip for a pane-local surface, with an always-on hairline.
///
/// [`button_on_surface`] paints no border and no resting fill, which is right
/// for a glyph — a `⌄` or a `⇥` is self-evidently a control — but wrong for a
/// word. On the agent-chat Activity Bar the chips sit beside the context
/// meter, which is static text in the same muted tone at the same size, so a
/// borderless chip is indistinguishable from a readout. Same fix, same reason
/// as [`button_status_pill`], which the status bar needed for the same
/// collision.
///
/// The border comes from the pane's own surface rather than the UI theme's
/// hairline: on a terminal-mirrored surface a fixed `t.border` is near
/// invisible (see [`button_on_surface`]). It uses `control_border`, not the
/// `border_tint` that edges cards — a card's edge is decoration, a control's
/// edge is what identifies it, and DESIGN.md holds that to 3:1.
pub fn button_chip_on_surface(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    surface: &PaneSurfaceTokens,
    cx: &App,
) -> Button {
    // The border has to come from the variant, not a `Styled` call: the render
    // registers per-state closures (hover / active / selected) that repaint
    // `border_color` on top of the resolved style, so a caller's border would
    // survive at rest and vanish the moment the pointer touched the chip.
    let variant = surface_button_variant(surface, cx).border(surface.control_border);
    button(id, label)
        .xsmall()
        .custom(variant)
        .rounded(px(theme::AGENT_CHAT_CHIP_RADIUS))
}

/// [`button_on_surface`] without a label — for the icon-only controls whose
/// glyph inherits the button's foreground.
pub fn button_bare_on_surface(
    id: impl Into<ElementId>,
    surface: &PaneSurfaceTokens,
    cx: &App,
) -> Button {
    button_bare(id).custom(surface_button_variant(surface, cx))
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

/// Status-bar pill button — the dropdown trigger shape shared by the
/// account slot (focused pane's account, or "System") and the Ports
/// segment (listening-port count). `ghost()` paints no border in any
/// state (see `ButtonVariant::Ghost::border_color`), which left it
/// reading as plain text next to the status bar's other muted labels;
/// this bakes an always-on hairline border plus a fixed compact height
/// so it reads as a clickable control at rest, not just on hover.
pub fn button_status_pill(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Button {
    button_status_pill_bare(id, cx).label(label)
}

/// [`button_status_pill`] without a label — for a pill that composes its
/// own spans as children because parts of it colour independently (the
/// usage chip tints only its percentage). The button paints `text_color`
/// from its variant on the outer container, so child spans inherit the
/// neutral `text_muted` unless they set their own colour.
pub fn button_status_pill_bare(id: impl Into<ElementId>, cx: &App) -> Button {
    let t = theme::current(cx);
    let variant = ButtonCustomVariant::new(cx)
        .foreground(t.text_muted)
        .border(t.border)
        .hover(t.status_bar_account_hover_bg);
    Button::new(id)
        .xsmall()
        .tab_stop(false)
        .custom(variant)
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
