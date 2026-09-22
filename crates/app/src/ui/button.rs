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
use gpui::{App, ElementId, ParentElement as _, SharedString, Styled as _, px};
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

/// Labelled action with leading 16px artwork, independent of the button tier.
pub fn button_with_icon(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    path: &'static str,
) -> Button {
    button_bare(id).child(super::icons::icon(path)).child(
        gpui::div()
            .flex_none()
            .line_height(gpui::relative(1.))
            .child(label.into()),
    )
}

/// Ghost chrome with a fixed 24px hit target and independent 16px SVG.
pub fn button_icon(id: impl Into<ElementId>, path: &'static str, cx: &App) -> Button {
    let t = theme::current(cx);
    icon_button_shell(id, path).custom(
        ButtonCustomVariant::new(cx)
            .foreground(t.text_muted)
            .hover(t.dock_icon_active_bg)
            .active(t.dock_icon_active_bg),
    )
}

/// Destructive action: neutral at rest, a translucent red fill on hover.
pub fn button_icon_danger(id: impl Into<ElementId>, path: &'static str, cx: &App) -> Button {
    let hover = theme::with_alpha(theme::ERROR, theme::CONTROL_DANGER_HOVER_ALPHA);
    icon_button_shell(id, path).custom(
        ButtonCustomVariant::new(cx)
            .foreground(theme::current(cx).text_muted)
            .hover(hover)
            .active(hover),
    )
}

/// The same metrics on a terminal-mirrored surface, with pane-local colours.
pub fn button_icon_on_surface(
    id: impl Into<ElementId>,
    path: &'static str,
    surface: &PaneSurfaceTokens,
    cx: &App,
) -> Button {
    icon_button_shell(id, path).custom(surface_button_variant(surface, cx))
}

fn icon_button_shell(id: impl Into<ElementId>, path: &'static str) -> Button {
    // The vendor's icon slot inherits Small (14px). A child decouples artwork
    // from the button tier while the shell still owns focus and disabled state.
    button_bare(id)
        .child(super::icons::icon(path))
        .w(px(theme::CONTROL_TARGET_SIZE))
        .h(px(theme::CONTROL_TARGET_SIZE))
        .p(px(0.))
        .rounded(px(theme::RADIUS_SM))
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

/// Neutral dismissal, distinct from deleting an item.
pub fn button_close(id: impl Into<ElementId>, cx: &App) -> Button {
    button_icon(id, super::icons::CLOSE, cx).tooltip(crate::surface::strings::common_button_close())
}

/// Delete action; the caller owns any row-hover visibility gating.
pub fn button_delete_glyph(id: impl Into<ElementId>, cx: &App) -> Button {
    button_icon_danger(id, super::icons::DELETE, cx)
        .tooltip(crate::surface::strings::common_button_delete())
}

/// Edit action, sharing the delete action's metrics without its danger tone.
pub fn button_edit_glyph(id: impl Into<ElementId>, cx: &App) -> Button {
    button_icon(id, super::icons::EDIT, cx)
}

/// Undo the in-progress queue edit, without deleting the queued item.
pub fn button_edit_cancel_glyph(id: impl Into<ElementId>, cx: &App) -> Button {
    button_icon(id, super::icons::UNDO, cx)
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
        .child(super::icons::icon(super::icons::ADD))
        .w(px(theme::BUTTON_WIDGET_HEIGHT))
        .h(px(theme::BUTTON_WIDGET_HEIGHT))
        .p(px(0.))
        .rounded(px(theme::BUTTON_WIDGET_RADIUS))
        .border(px(theme::BUTTON_WIDGET_ADD_BORDER_W))
        .border_dashed()
        .text_size(px(theme::BUTTON_WIDGET_FONT_SIZE))
}

/// A window caption control (minimize / maximize / restore / close) drawn by
/// the app because the platform removed the OS caption. Square-ish and
/// flush to the title bar edge, Windows-style; `danger` gives close its red
/// hover without painting it red at rest.
pub fn button_window_control(
    id: impl Into<ElementId>,
    icon_path: &'static str,
    cx: &App,
) -> Button {
    window_control_shell(id, icon_path, false, cx)
}

/// The close control. Its own factory rather than a `danger` flag on the one
/// above — `ui/CLAUDE.md` holds that a variant reads better as a name at the
/// call site than as a bool argument, the way `button` / `button_danger` do.
pub fn button_window_control_danger(
    id: impl Into<ElementId>,
    icon_path: &'static str,
    cx: &App,
) -> Button {
    window_control_shell(id, icon_path, true, cx)
}

fn window_control_shell(
    id: impl Into<ElementId>,
    icon_path: &'static str,
    danger: bool,
    cx: &App,
) -> Button {
    let t = theme::current(cx);
    let hover_bg = if danger {
        theme::with_alpha(theme::ERROR, theme::CONTROL_DANGER_HOVER_ALPHA)
    } else {
        t.dock_icon_active_bg
    };
    let variant = ButtonCustomVariant::new(cx)
        .foreground(t.text_muted)
        .hover(hover_bg)
        .active(hover_bg);
    Button::new(id)
        .small()
        .tab_stop(false)
        .custom(variant)
        .child(super::icons::icon(icon_path))
        .w(px(theme::WINDOW_CONTROL_W))
        .h(px(theme::TITLE_BAR_HEIGHT))
        .p(px(0.))
        .rounded(px(0.))
        .text_size(px(theme::WINDOW_CONTROL_GLYPH_SIZE))
}

/// Dock toggle: the caller chooses filled artwork for the active state.
pub fn button_toggle_icon(
    id: impl Into<ElementId>,
    icon_path: &'static str,
    active: bool,
    cx: &App,
) -> Button {
    toggle_shell(id, active, cx)
        .child(super::icons::icon(icon_path).with_size(px(theme::DOCK_TOGGLE_ICON_SIZE)))
}

/// Active state brightens the filled SVG, without a persistent button fill.
fn toggle_shell(id: impl Into<ElementId>, active: bool, cx: &App) -> Button {
    let t = theme::current(cx);
    let fg = if active { t.text_primary } else { t.text_muted };
    let active_bg = t.dock_icon_active_bg;
    let variant = ButtonCustomVariant::new(cx)
        .foreground(fg)
        .hover(active_bg)
        .active(active_bg);
    Button::new(id)
        .small()
        .tab_stop(false)
        .custom(variant)
        .w(px(theme::DOCK_ICON_BUTTON_W))
        .h(px(theme::DOCK_ICON_BUTTON_H))
        .p(px(0.))
        .rounded(px(theme::DOCK_ICON_BUTTON_RADIUS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::Disableable as _;
    use gpui::{
        Context, InteractiveElement as _, IntoElement, Render, TestAppContext, VisualTestContext,
        Window, div, size,
    };

    #[derive(Default)]
    struct ChromeProbe {
        presses: usize,
    }

    impl Render for ChromeProbe {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .flex_col()
                .items_start()
                .child(
                    button_icon("live", crate::ui::icons::ADD, cx)
                        .debug_selector(|| "chrome-live".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.presses += 1;
                            cx.notify();
                        })),
                )
                .child(
                    button_icon_danger("off", crate::ui::icons::DELETE, cx)
                        .debug_selector(|| "chrome-disabled".into())
                        .disabled(true)
                        .on_click(cx.listener(|this, _, _, _| this.presses += 1)),
                )
                .child(
                    crate::ui::tab_bar("tabs")
                        .child(crate::ui::tab("Tab").debug_selector(|| "chrome-tab".into())),
                )
                .child(
                    button_with_icon("new", "New", crate::ui::icons::ADD)
                        .primary()
                        .xsmall()
                        .debug_selector(|| "chrome-labelled".into()),
                )
        }
    }

    #[gpui::test]
    fn chrome_targets_fit_the_tab_grid_and_disabled_controls_ignore_clicks(
        cx: &mut TestAppContext,
    ) {
        crate::test_support::init_gpui_component(cx);
        let window = cx.add_window(|_, _| ChromeProbe::default());
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let live = vcx
            .debug_bounds("chrome-live")
            .expect("live button painted");
        let disabled = vcx
            .debug_bounds("chrome-disabled")
            .expect("disabled button painted");
        let target = px(theme::CONTROL_TARGET_SIZE);
        assert_eq!(live.size, size(target, target));
        assert_eq!(disabled.size, size(target, target));
        assert_eq!(
            vcx.debug_bounds("chrome-tab").unwrap().size.height,
            px(theme::TAB_BAR_HEIGHT)
        );
        let labelled = vcx.debug_bounds("chrome-labelled").unwrap();
        assert!(labelled.size.height >= px(theme::CONTROL_ICON_SIZE));
        assert!(labelled.size.width > labelled.size.height);
        vcx.simulate_click(live.center(), Default::default());
        vcx.run_until_parked();
        vcx.simulate_click(disabled.center(), Default::default());
        vcx.run_until_parked();
        assert_eq!(window.read_with(&vcx, |view, _| view.presses).unwrap(), 1);
    }
}
