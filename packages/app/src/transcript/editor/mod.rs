//! The Fold, Filter and Range editors, and the chrome they share.
//!
//! All are host-neutral: they take the value, an id namespace, a type size and
//! a set of callbacks, and know nothing about who is showing them. The chat
//! pane opens them in its Activity Bar popover to change what one pane shows;
//! the Settings agent catalog opens the same editors to author the defaults a pane
//! starts on. Neither can drift from the other, because there is only one.

use std::rc::Rc;

use gpui::{App, Context, Div, Hsla, IntoElement, SharedString, Window, div, prelude::*, px};

use crate::ui::theme;
use crate::ui::{Icon, PopoverState, Sizable as _};

pub(crate) mod filter;
pub(crate) mod fold;
pub(crate) mod range;
pub(crate) mod state;

/// What the reset button runs.
pub(crate) type ResetPress = Rc<dyn Fn(&mut Window, &mut App)>;

/// The footer: where the axis value comes from, and the command that hands it
/// back. Each host names both — a pane against the agent's stated value, an
/// agent row against the built-in. `overridden` is the host's call: a value
/// that merely *equals* the target can still be an override worth undoing.
pub(crate) struct ResetSpec {
    pub label: String,
    pub source: String,
    pub overridden: bool,
    pub on_reset: ResetPress,
    /// Keeping the override instead, as the default it departed from — the
    /// chat pane's "save as agent default". Offered only while overridden.
    pub save: Option<SaveSpec>,
}

/// The footer's save command: its label and what it runs.
pub(crate) struct SaveSpec {
    pub label: String,
    pub on_save: ResetPress,
}

/// The editor's three text roles, derived from one base size so a configured
/// font scales the title, the rows and the auxiliary lines together.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TextRoles {
    pub title: f32,
    pub body: f32,
    pub aux: f32,
}

impl TextRoles {
    pub(crate) fn from_base(base: f32) -> Self {
        Self {
            title: base,
            body: base * theme::TRANSCRIPT_EDITOR_BODY_RATIO,
            aux: base * theme::TRANSCRIPT_EDITOR_AUX_RATIO,
        }
    }
}

/// The panel the editors sit in. The host strips the popover's own padding
/// (`.p_0()`), so this box plus the popover border is the whole panel and
/// [`theme::TRANSCRIPT_EDITOR_RULES_PANEL_W`] names its outer width once. The
/// height stops at the window edge below the trigger, so the vendored snap
/// does not lift the panel over the control that opened it.
pub(crate) fn panel_root(window: &Window, state: &PopoverState) -> Div {
    let viewport = window.viewport_size();
    let below = state
        .trigger_bounds()
        .map(|bounds| f32::from(bounds.bottom()) + theme::TRANSCRIPT_EDITOR_TRIGGER_GAP);
    let border = 2.0 * theme::TRANSCRIPT_EDITOR_PANEL_BORDER;
    div()
        .debug_selector(|| "transcript-editor-panel".into())
        .w(px(panel_outer_w(f32::from(viewport.width)) - border))
        .max_h(px(
            panel_outer_max_h(f32::from(viewport.height), below) - border
        ))
        .overflow_hidden()
        .flex()
        .flex_col()
}

/// The outer width: the design width, or the window less both margins.
fn panel_outer_w(viewport_w: f32) -> f32 {
    theme::TRANSCRIPT_EDITOR_RULES_PANEL_W
        .min(viewport_w - 2.0 * theme::TRANSCRIPT_EDITOR_WINDOW_MARGIN)
        .max(0.0)
}

/// The outer height cap. `top` is where the panel opens; without one (the
/// trigger has not painted yet) the whole window less its margins is the room.
fn panel_outer_max_h(viewport_h: f32, top: Option<f32>) -> f32 {
    let margin = theme::TRANSCRIPT_EDITOR_WINDOW_MARGIN;
    let room = match top {
        Some(top) => viewport_h - top - margin,
        None => viewport_h - 2.0 * margin,
    };
    let floor = theme::TRANSCRIPT_EDITOR_MIN_PANEL_H.min(viewport_h - 2.0 * margin);
    theme::TRANSCRIPT_EDITOR_RULES_PANEL_MAX_H
        .min(room.max(floor))
        .max(0.0)
}

/// Dismisses the popover a panel sits in — the header's close button.
///
/// The same two steps the vendored popover runs on an outside press: shut the
/// state, then repaint the view that renders the trigger.
pub(crate) fn dismiss_press(window: &Window, cx: &mut Context<PopoverState>) -> ResetPress {
    let state = cx.entity().downgrade();
    let host = window.current_view();
    Rc::new(move |window, app| {
        if let Some(state) = state.upgrade() {
            state.update(app, |state, cx| state.dismiss(window, cx));
        }
        app.notify(host);
    })
}

/// Title and close button above everything else in the panel. Where the values
/// come from is the footer's to say, so the header does not repeat it.
pub(crate) fn panel_header(
    id: SharedString,
    title: String,
    text: TextRoles,
    on_close: ResetPress,
    cx: &App,
) -> Div {
    let t = theme::current(cx);
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(theme::GAP_LG))
        .pt(px(theme::TRANSCRIPT_EDITOR_HEADER_PAD_TOP))
        .pb(px(theme::TRANSCRIPT_EDITOR_HEADER_PAD_BOTTOM))
        .px(px(theme::TRANSCRIPT_EDITOR_PAD_X))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(text.title))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.text_primary)
                .child(SharedString::from(title)),
        )
        .child(
            crate::ui::button_close(id, cx)
                .debug_selector(|| "transcript-editor-close".into())
                .tab_stop(true)
                .on_click(move |_, window, app| on_close(window, app)),
        )
}

/// The tab strip's band: full width, its hairline under the tabs.
pub(crate) fn tabs_band(tabs: impl IntoElement, cx: &App) -> Div {
    div()
        .flex_none()
        .px(px(theme::TRANSCRIPT_EDITOR_TABS_PAD_X))
        .border_b_1()
        .border_color(theme::current(cx).border)
        .child(tabs)
}

/// An icon-led auxiliary line — a scope, a heading or a source status. The
/// glyph takes its own tone so it can carry a state the text stays quiet about.
pub(crate) fn aux_line(
    icon: &'static str,
    icon_color: Hsla,
    label: String,
    color: Hsla,
    text: TextRoles,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(theme::GAP_STANDARD))
        .min_w_0()
        .text_size(px(text.aux))
        .text_color(color)
        .child(aux_icon(icon, icon_color))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(SharedString::from(label)),
        )
}

pub(crate) fn aux_icon(icon: &'static str, color: Hsla) -> Icon {
    Icon::empty()
        .path(icon)
        .with_size(px(theme::TRANSCRIPT_EDITOR_AUX_ICON))
        .text_color(color)
}

/// The band a panel scrolls: rules and facets outgrow the popover, the header,
/// cleanup band and footer must not move when they do. Its thumb rides the
/// body's right inset.
pub(crate) fn scroll_region(id: impl Into<SharedString>, content: Div) -> crate::ui::ScrollArea {
    crate::ui::scroll_area(id, px(0.), content).fill()
}

/// What [`scroll_region`] scrolls: the body column, inset on both axes.
pub(crate) fn scroll_content() -> Div {
    div()
        .flex()
        .flex_col()
        .px(px(theme::TRANSCRIPT_EDITOR_PAD_X))
        .py(px(theme::TRANSCRIPT_EDITOR_BODY_PAD_Y))
}

/// A full-width band pinned below the scrolling body, its hairline on top.
pub(crate) fn fixed_band(cx: &App) -> Div {
    div()
        .flex_none()
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(theme::current(cx).border)
        .px(px(theme::TRANSCRIPT_EDITOR_PAD_X))
}

/// The column an editor renders into the panel: its body, then its bands.
pub(crate) fn editor_column(text: TextRoles) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .flex()
        .flex_col()
        .text_size(px(text.body))
}

/// The footer each editor ends with — source status on the left, the hand-back
/// command on the right — or nothing in a host that offers no return.
pub(crate) fn reset_footer(
    id: SharedString,
    reset: Option<ResetSpec>,
    text: TextRoles,
    cx: &App,
) -> Div {
    let Some(reset) = reset else {
        return div().flex_none();
    };
    use crate::ui::{ButtonVariants as _, Disableable as _, icons};
    let t = theme::current(cx);
    let (icon, color) = if reset.overridden {
        (icons::PIN, theme::PRIMARY)
    } else {
        (icons::AGENT, t.text_muted)
    };
    div()
        .flex_none()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(theme::GAP_LG))
        .min_h(px(theme::TRANSCRIPT_EDITOR_FOOTER_MIN_H))
        .px(px(theme::TRANSCRIPT_EDITOR_FOOTER_PAD_X))
        .py(px(theme::TRANSCRIPT_EDITOR_FOOTER_PAD_Y))
        .border_t_1()
        .border_color(t.border)
        // The pin carries an override in the accent; the text keeps the muted
        // tone so the line reads as status rather than as a second control.
        .child(div().flex_1().min_w_0().child(aux_line(
            icon,
            color,
            reset.source,
            t.text_muted,
            text,
        )))
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(theme::GAP_SM))
                .when_some(reset.save.filter(|_| reset.overridden), |commands, save| {
                    commands.child(
                        crate::ui::button_bare(SharedString::from(format!("{id}-save")))
                            .ghost()
                            .tab_stop(true)
                            .icon(Icon::empty().path(icons::SAVE))
                            .child(
                                div()
                                    .text_size(px(text.aux))
                                    .child(SharedString::from(save.label)),
                            )
                            .on_click(move |_, window, app| (save.on_save)(window, app)),
                    )
                })
                .child(
                    crate::ui::button_bare(id)
                        .ghost()
                        .tab_stop(true)
                        .icon(Icon::empty().path(icons::UNDO))
                        .child(
                            div()
                                .text_size(px(text.aux))
                                .child(SharedString::from(reset.label)),
                        )
                        .disabled(!reset.overridden)
                        .on_click(move |_, window, app| (reset.on_reset)(window, app)),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::{TextRoles, panel_outer_max_h, panel_outer_w};
    use crate::ui::theme;

    const MARGIN: f32 = theme::TRANSCRIPT_EDITOR_WINDOW_MARGIN;

    #[test]
    fn the_design_width_is_the_outer_box_in_any_roomy_window() {
        for viewport in [800.0_f32, 1280.0, 3840.0] {
            assert_eq!(
                panel_outer_w(viewport),
                theme::TRANSCRIPT_EDITOR_RULES_PANEL_W
            );
        }
    }

    #[test]
    fn a_narrow_window_keeps_its_margin_on_both_sides() {
        for viewport in [320.0_f32, 400.0, 470.0] {
            let w = panel_outer_w(viewport);
            assert!(w < theme::TRANSCRIPT_EDITOR_RULES_PANEL_W, "{viewport}");
            assert_eq!(w + 2.0 * MARGIN, viewport, "{viewport}");
        }
    }

    #[test]
    fn the_height_stops_at_the_window_edge_below_the_trigger() {
        // Tall window: the design cap binds.
        assert_eq!(
            panel_outer_max_h(1200.0, Some(80.0)),
            theme::TRANSCRIPT_EDITOR_RULES_PANEL_MAX_H
        );
        // Short window: what is left under the trigger binds, so the vendored
        // snap has no reason to lift the panel over the trigger.
        let top = 100.0;
        let viewport = 500.0;
        assert_eq!(
            panel_outer_max_h(viewport, Some(top)),
            viewport - top - MARGIN
        );
        // Before the trigger has painted, the window less both margins.
        assert_eq!(panel_outer_max_h(500.0, None), 500.0 - 2.0 * MARGIN);
    }

    #[test]
    fn a_trigger_near_the_bottom_still_opens_a_usable_panel() {
        let h = panel_outer_max_h(900.0, Some(800.0));
        assert_eq!(h, theme::TRANSCRIPT_EDITOR_MIN_PANEL_H);
        // A trigger already off the bottom edge (a scrolled-away field).
        assert_eq!(
            panel_outer_max_h(900.0, Some(1400.0)),
            theme::TRANSCRIPT_EDITOR_MIN_PANEL_H
        );
        // The floor never outgrows the window itself.
        assert!(panel_outer_max_h(200.0, Some(150.0)) <= 200.0 - 2.0 * MARGIN);
    }

    #[test]
    fn text_roles_hold_the_default_sizes_and_scale_together() {
        let base = TextRoles::from_base(13.0);
        assert_eq!(base.title, 13.0);
        assert!((base.body - 12.0).abs() < 1e-4);
        assert!((base.aux - 11.0).abs() < 1e-4);
        let large = TextRoles::from_base(26.0);
        assert!((large.body / base.body - 2.0).abs() < 1e-4);
        assert!((large.aux / base.aux - 2.0).abs() < 1e-4);
    }

    mod shell {
        use std::cell::RefCell;
        use std::rc::Rc;

        use gpui::{
            AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Render,
            SharedString, Styled as _, TestAppContext, VisualTestContext, Window, WindowBounds,
            WindowOptions, div, point, px, size,
        };

        use super::super::{TextRoles, dismiss_press, panel_header, panel_root};
        use crate::surface::strings as s;
        use crate::ui::{Popover, PopoverState, theme};

        type Slot = Rc<RefCell<Option<Entity<PopoverState>>>>;

        /// A trigger whose popover opens on the shared shell, as both hosts do.
        struct ShellProbe {
            slot: Slot,
        }

        impl Render for ShellProbe {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let slot = self.slot.clone();
                div().size_full().child(
                    Popover::new(SharedString::from("shell-probe"))
                        .default_open(true)
                        .p_0()
                        .trigger(crate::ui::button("shell-trigger", "View"))
                        .content(move |state, window, cx| {
                            *slot.borrow_mut() = Some(cx.entity());
                            let close = dismiss_press(window, cx);
                            panel_root(window, state)
                                .child(panel_header(
                                    SharedString::from("shell-close"),
                                    s::agent_chat::view_options_title(),
                                    TextRoles::from_base(theme::AGENT_CHAT_MSG_FONT_SIZE),
                                    close,
                                    cx,
                                ))
                                .into_any_element()
                        }),
                )
            }
        }

        fn open(cx: &mut TestAppContext) -> (VisualTestContext, Slot) {
            crate::test_support::init_gpui_component(cx);
            let slot: Slot = Rc::default();
            let probe = slot.clone();
            let bounds = Bounds::new(point(px(0.), px(0.)), size(px(1280.), px(900.)));
            let opts = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            };
            let window = cx
                .update(|cx| cx.open_window(opts, |_, cx| cx.new(|_| ShellProbe { slot: probe })))
                .expect("window opens");
            let mut vcx = VisualTestContext::from_window(window.into(), cx);
            vcx.run_until_parked();
            vcx.update(|window, _| window.refresh());
            vcx.run_until_parked();
            (vcx, slot)
        }

        /// The design width is the outer box: the panel plus the popover's own
        /// border, and no popover padding around either.
        #[gpui::test]
        fn the_panel_and_its_border_are_the_design_width(cx: &mut TestAppContext) {
            let (mut vcx, _) = open(cx);
            let panel = vcx
                .debug_bounds("transcript-editor-panel")
                .expect("panel painted");
            let outer = f32::from(panel.size.width) + 2.0 * theme::TRANSCRIPT_EDITOR_PANEL_BORDER;
            assert_eq!(outer, theme::TRANSCRIPT_EDITOR_RULES_PANEL_W);
            // The popover opens at the trigger's left edge; only its border
            // sits between that edge and the panel, no padding.
            assert_eq!(
                f32::from(panel.origin.x),
                theme::TRANSCRIPT_EDITOR_PANEL_BORDER
            );
        }

        /// The header's close button shuts the popover it sits in — the same
        /// state an outside press or Escape leaves.
        #[gpui::test]
        fn the_close_button_dismisses_the_popover(cx: &mut TestAppContext) {
            let (mut vcx, slot) = open(cx);
            let state = slot.borrow().clone().expect("content rendered");
            assert!(vcx.update(|_, cx| state.read(cx).is_open()));
            let close = vcx
                .debug_bounds("transcript-editor-close")
                .expect("close painted");
            vcx.simulate_click(close.center(), Default::default());
            vcx.run_until_parked();
            assert!(!vcx.update(|_, cx| state.read(cx).is_open()));
            assert!(
                vcx.debug_bounds("transcript-editor-panel").is_none(),
                "the panel is gone from the next frame"
            );
        }
    }
}
