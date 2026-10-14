//! Shared chrome for the status bar's chip popups: each opens above the bar
//! and reads header, separator, body, then — for menus — a footer action.
//! Panels carry the menu's padding in their content, separators edge to edge.

use crate::ui::theme;
use crate::ui::{Button, Divider, DropdownMenu as _, Popover, PopupMenu, PopupMenuItem};
use gpui::{
    Anchor, AnyElement, App, ClickEvent, Context, Div, Hsla, IntoElement, SharedString, Window,
    div, prelude::*, px,
};

/// Every popup-bearing chip sits in the bar's right cluster at the window's
/// bottom edge, so popups grow up and left from the chip rather than being
/// pushed back over the bar by the window-edge snap.
pub(super) const ANCHOR: Anchor = Anchor::BottomRight;

/// The `PopupMenu` edge padding (`p_1`), reproduced inside panels; tracks
/// the vendored value.
const EDGE_PAD: f32 = theme::PAD_XS;
/// Where a menu item's text starts: the edge padding plus the item's own.
const ROW_INSET: f32 = EDGE_PAD + theme::PAD_STANDARD;
/// A panel's border, both sides; its padding lives in the content.
pub(super) const PANEL_CHROME: f32 = 2.0 * theme::INPUT_BORDER_WIDTH;
/// A scrolling panel body's left inset; its right side is the scrollbar's.
pub(super) const PANEL_BODY_INSET: f32 = ROW_INSET;
/// The vendored `Popover`'s gap between chip and popup (`bottom_1`) and its
/// window-edge snap margin; both track the vendor values.
const ANCHOR_GAP: f32 = theme::PAD_XS;
const SNAP_MARGIN: f32 = theme::PAD_STANDARD;

/// The tallest popup that fits between the window's top snap margin and the
/// bar; a taller one is snapped back down over the chip.
fn room_above_bar(window: &Window) -> f32 {
    let viewport_h = f32::from(window.viewport_size().height);
    (viewport_h - super::STATUS_BAR_HEIGHT - ANCHOR_GAP - SNAP_MARGIN).max(0.0)
}

/// A chip's dropdown menu, opened above the chip. `shot_id` names it to the
/// screenshot harness, which opens it without a click.
pub(super) fn dropdown(
    shot_id: &'static str,
    trigger: Button,
    builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    cx: &App,
) -> AnyElement {
    // Past the room above the bar the menu scrolls rather than covering it.
    let builder = move |menu, window: &mut Window, cx: &mut Context<PopupMenu>| {
        let max_h = (room_above_bar(window) - PANEL_CHROME).max(0.0);
        builder(menu, window, cx).max_h(px(max_h)).scrollable(true)
    };
    #[cfg(feature = "screenshot")]
    if super::screenshot::is_open(shot_id, cx) {
        return super::screenshot::open_menu(shot_id, trigger, builder);
    }
    #[cfg(not(feature = "screenshot"))]
    let _ = (shot_id, cx);
    trigger
        .dropdown_menu_with_anchor(ANCHOR, builder)
        .into_any_element()
}

/// A chip's browsing panel, opened above the chip. Its content carries the
/// padding: rows through [`panel_row`] / [`panel_body`].
pub(super) fn popover(id: &'static str, cx: &App) -> Popover {
    let popover = Popover::new(id).anchor(ANCHOR).p_0();
    #[cfg(feature = "screenshot")]
    let popover = super::screenshot::open_for_shot(popover, id, cx);
    #[cfg(not(feature = "screenshot"))]
    let _ = cx;
    popover
}

/// A panel's sized column: `content_width` wide inside the body inset and
/// chrome, its body at most `body_max` tall, the whole within the room above
/// the bar so a short window scrolls the body instead of covering the chip.
pub(super) fn panel(window: &Window, content_width: f32, body_max: f32) -> Div {
    let natural = body_max + theme::POPOVER_HEADER_BUDGET + PANEL_CHROME;
    crate::ui::popover_panel(
        window,
        crate::ui::PanelLimits {
            width: content_width + PANEL_BODY_INSET + PANEL_CHROME,
            max_height: natural.min(room_above_bar(window)),
            min_height: 0.0,
            margin: theme::POPOVER_WINDOW_MARGIN,
            chrome: PANEL_CHROME,
        },
        None,
    )
}

/// How a header's state reads. Resolved at render: a menu is built once per
/// open, and a theme swap must still repaint its header.
#[derive(Clone, Copy)]
pub(super) enum Tone {
    /// The header's own colour.
    Plain,
    /// A colour that does not follow the theme (usage severity).
    Fixed(Hsla),
    /// A theme slot that asks for attention.
    Themed(fn(&App) -> Hsla),
}

impl Tone {
    fn resolve(self, cx: &App) -> Option<Hsla> {
        match self {
            Self::Plain => None,
            Self::Fixed(color) => Some(color),
            Self::Themed(slot) => Some(slot(cx)),
        }
    }
}

/// Title left, state right; the state truncates before the title does.
fn header_line(
    title: SharedString,
    state: Option<SharedString>,
    state_tone: Tone,
    cx: &App,
) -> Div {
    let state_tone = state_tone.resolve(cx);
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(theme::GAP_LG))
        .text_size(px(theme::FONT_SIZE_SM))
        .child(div().flex_none().child(title))
        .children(state.map(|state| {
            div()
                .min_w_0()
                .truncate()
                .when_some(state_tone, |el, tone| el.text_color(tone))
                .child(state)
        }))
        .debug_selector(|| "status-menu-header".into())
}

/// A menu's header item: inert, at the shared menu width.
pub(super) fn menu_header(
    title: impl Into<SharedString>,
    state: Option<SharedString>,
    state_tone: Tone,
) -> PopupMenuItem {
    let title = title.into();
    PopupMenuItem::element(move |_window, cx| {
        header_line(title.clone(), state.clone(), state_tone, cx)
            .w(px(theme::STATUS_BAR_MENU_WIDTH))
            .into_any_element()
    })
    .disabled(true)
}

/// The separator and the single footer action that close every menu.
pub(super) fn with_footer(
    menu: PopupMenu,
    label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> PopupMenu {
    menu.separator()
        .item(PopupMenuItem::new(label.into()).on_click(on_click))
}

/// A panel's header row, padded like a menu's first item.
pub(super) fn panel_header(
    title: impl Into<SharedString>,
    state: Option<SharedString>,
    state_tone: Tone,
    cx: &App,
) -> AnyElement {
    let t = theme::current(cx);
    panel_row(
        header_line(title.into(), state, state_tone, cx)
            .w_full()
            .text_color(t.text_muted),
    )
    .pt(px(EDGE_PAD + theme::GAP_XS))
    .pb(px(theme::GAP_XS))
    .into_any_element()
}

/// A border-to-border separator, the panel twin of `PopupMenu::separator`.
pub(super) fn panel_separator() -> AnyElement {
    div()
        .flex_none()
        .child(Divider::horizontal())
        .into_any_element()
}

/// Wraps panel content in a menu item's horizontal inset.
pub(super) fn panel_row(child: impl IntoElement) -> Div {
    div().flex_none().px(px(ROW_INSET)).child(child)
}

/// A panel's scrolling body: the row inset on the left only, so the
/// scrollbar sits at the panel's edge. Takes the panel's remaining height.
pub(super) fn panel_body(child: impl IntoElement) -> Div {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .pl(px(PANEL_BODY_INSET))
        .pb(px(EDGE_PAD))
        .child(child)
}

/// The leading column every body row reserves, so names align whether the
/// row carries a selection dot, an icon, or nothing.
fn gutter(mark: Option<AnyElement>) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .w(px(theme::CONTROL_ICON_SIZE))
        .children(mark)
}

fn selection_dot(cx: &App) -> AnyElement {
    div()
        .w(px(theme::STATUS_BAR_DOT_SIZE))
        .h(px(theme::STATUS_BAR_DOT_SIZE))
        .rounded_full()
        .bg(theme::current(cx).text_primary)
        .into_any_element()
}

/// One choice: the dot gutter, then its name over an optional muted,
/// wrapping description. The dot marks the current choice.
pub(super) fn choice_row(
    name: SharedString,
    description: Option<SharedString>,
    selected: bool,
    cx: &App,
) -> AnyElement {
    let t = theme::current(cx);
    div()
        .flex()
        .flex_row()
        .items_center()
        .w(px(theme::STATUS_BAR_MENU_WIDTH))
        .py(px(theme::PAD_SM))
        .child(gutter(selected.then(|| selection_dot(cx))))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().truncate().child(name))
                .children(description.map(|description| {
                    div()
                        .text_size(px(theme::FONT_SIZE_SM))
                        .text_color(t.text_muted)
                        .child(description)
                })),
        )
        .into_any_element()
}

/// An action or note aligned with the choice names; `mark` (an agent icon,
/// a spinner) sits in the gutter.
pub(super) fn action_row(mark: Option<AnyElement>, label: SharedString) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .w(px(theme::STATUS_BAR_MENU_WIDTH))
        .child(gutter(mark))
        .child(div().flex_1().min_w_0().child(label))
        .into_any_element()
}

/// A muted group heading inside a menu body; `mark` sits in the gutter.
pub(super) fn section_row(mark: Option<AnyElement>, label: SharedString, cx: &App) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .w(px(theme::STATUS_BAR_MENU_WIDTH))
        .text_size(px(theme::FONT_SIZE_SM))
        .text_color(theme::current(cx).text_muted)
        .child(gutter(mark))
        .child(div().flex_1().min_w_0().truncate().child(label))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Bounds, Modifiers, TestAppContext, VisualTestContext, point, size};

    /// What the probe chip opens.
    #[derive(Clone, Copy)]
    enum Popup {
        Menu,
        Panel,
        /// A panel whose body holds far more rows than any window shows.
        TallPanel,
    }

    /// A chip in a bar-high strip at the window's bottom-right corner, the way
    /// the bar places it.
    struct CornerProbe {
        popup: Popup,
    }

    impl gpui::Render for CornerProbe {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let trigger = crate::ui::button_status_pill_bare("probe-chip", cx)
                .child("chip")
                .debug_selector(|| "probe-chip".into());
            let popup = match self.popup {
                Popup::Menu => dropdown(
                    "probe-menu",
                    trigger,
                    crate::ui::menu_builder(|menu, _window, _cx| {
                        menu.item(menu_header("Title", Some("State".into()), Tone::Plain))
                    }),
                    cx,
                ),
                Popup::Panel => popover("probe-panel", cx)
                    .trigger(trigger)
                    .content(|_, _window, cx| {
                        panel_header("Title", Some("State".into()), Tone::Plain, cx)
                    })
                    .into_any_element(),
                Popup::TallPanel => popover("probe-tall-panel", cx)
                    .trigger(trigger)
                    .content(|_, window, cx| {
                        let rows = div()
                            .flex()
                            .flex_col()
                            .children((0..80).map(|ix| panel_row(format!("row {ix}"))));
                        panel(window, 200.0, 360.0)
                            .debug_selector(|| "probe-popup".into())
                            .child(panel_header("Title", None, Tone::Plain, cx))
                            .child(panel_separator())
                            .child(panel_body(
                                crate::ui::scroll_area("probe-scroll", px(360.0), rows)
                                    .fill()
                                    .max_h(px(360.0)),
                            ))
                    })
                    .into_any_element(),
            };
            div()
                .size_full()
                .flex()
                .flex_col()
                .justify_end()
                .items_end()
                .child(
                    div()
                        .h(px(super::super::STATUS_BAR_HEIGHT))
                        .flex()
                        .items_center()
                        .child(popup),
                )
        }
    }

    fn open_at_corner(
        cx: &mut TestAppContext,
        popup: Popup,
        window_h: f32,
    ) -> (Bounds<gpui::Pixels>, VisualTestContext) {
        crate::test_support::init_gpui_component(cx);
        let opts = gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                size(px(800.), px(window_h)),
            ))),
            ..Default::default()
        };
        let window = cx
            .update(|cx| cx.open_window(opts, |_, cx| cx.new(|_| CornerProbe { popup })))
            .unwrap();
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        let chip = vcx.debug_bounds("probe-chip").expect("chip painted");
        vcx.simulate_click(chip.center(), Modifiers::none());
        vcx.run_until_parked();
        (chip, vcx)
    }

    /// A menu at the bottom edge opens upward instead of being snapped back
    /// over the chip that opened it.
    #[gpui::test]
    fn a_menu_opens_above_its_chip(cx: &mut TestAppContext) {
        let (chip, mut vcx) = open_at_corner(cx, Popup::Menu, 600.0);
        let header = vcx.debug_bounds("status-menu-header").expect("opened");
        assert!(header.bottom() <= chip.top(), "{header:?} covers {chip:?}");
    }

    #[gpui::test]
    fn a_panel_opens_above_its_chip(cx: &mut TestAppContext) {
        let (chip, mut vcx) = open_at_corner(cx, Popup::Panel, 600.0);
        let header = vcx.debug_bounds("status-menu-header").expect("opened");
        assert!(header.bottom() <= chip.top(), "{header:?} covers {chip:?}");
    }

    /// A full panel is taller than the room above the bar in a short window;
    /// it scrolls its body and keeps clear of both the chip and the top edge.
    #[gpui::test]
    fn a_tall_panel_stays_above_its_chip_in_a_short_window(cx: &mut TestAppContext) {
        let (chip, mut vcx) = open_at_corner(cx, Popup::TallPanel, 400.0);
        let popup = vcx.debug_bounds("probe-popup").expect("opened");
        assert!(popup.bottom() <= chip.top(), "{popup:?} covers {chip:?}");
        assert!(
            popup.top() >= px(SNAP_MARGIN),
            "{popup:?} runs into the window's top edge"
        );
    }
}
