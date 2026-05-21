//! Composed input: multi-line [`InputState`] + configurable action buttons.
//!
//! Wraps a `gpui_component::Input` (multi-line) with a list of
//! [`PanelAction`]s rendered either to the right of the area
//! (`ActionsRight`), below it (`ActionsBelow`), or floating along the
//! bottom edge (`ActionsFloating`). `InputEvent`s from the underlying
//! `InputState` are forwarded as [`InputPanelEvent`].
//!
//! Single-line callers go through `crate::ui::input` instead — this
//! composite exists because `bottom/terminal_input`, the git commit
//! box, and similar surfaces need an inline button bar bound to the
//! same text buffer.

use std::sync::Arc;

use crate::ui::theme;
use gpui::{
    AnyElement, App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, Render,
    SharedString, Subscription, Window, div, prelude::*, px,
};
use gpui_component::Sizable as _;

use super::{ButtonVariants as _, DropdownButton, PopupMenuItem};
use super::{Input, InputEvent, InputState};

/// Visual treatment for an action button rendered inside an
/// [`InputPanel`]. Mirrors the three modal-footer variants exposed by
/// `crate::ui::button*` factories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelActionVariant {
    /// Accent action (Send, Commit). Routed to `crate::ui::button_primary`.
    Primary,
    /// Subdued / neutral. Routed to `crate::ui::button` (secondary default).
    Secondary,
    /// Destructive. Routed to `crate::ui::button_danger`.
    Danger,
}

/// Layout of action buttons relative to the text area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPanelLayout {
    /// Text area on the left, buttons stacked vertically on the right.
    ActionsRight,
    /// Text area on top, button row below.
    ActionsBelow,
    /// Buttons float as an absolute 32px bar at the bottom of the text area.
    /// The text area gets bottom padding equal to the bar height so text is
    /// never hidden under the bar. All buttons are right-aligned in the bar.
    ActionsFloating,
}

/// Events emitted by [`InputPanel`].
#[derive(Debug, Clone, PartialEq)]
pub enum InputPanelEvent {
    /// User confirmed (`Cmd+Enter` — `InputEvent::PressEnter { secondary: true }`).
    Submit,
    /// Text changed.
    Changed,
}

/// A single entry inside a [`PanelAction`]'s dropdown menu. When a
/// `PanelAction` has any dropdown items, the panel renders it as a
/// split-button (primary action on the left, caret on the right).
#[derive(Clone)]
pub struct PanelDropdownItem {
    pub(super) label: SharedString,
    #[allow(clippy::type_complexity)]
    pub(super) on_select: Arc<dyn Fn(&mut Window, &mut App) + 'static>,
}

/// One action button registered on an [`InputPanel`].
///
/// `dropdown_items` is stored as `Rc<Vec<_>>` so the render path can
/// hand a cheap clone (one ref-count bump) to the popup-menu builder
/// closure without reallocating the inner `Vec` on every frame. GPUI
/// runs on a single thread, so the non-`Send` `Rc` is fine here.
pub struct PanelAction {
    pub(super) id: SharedString,
    pub(super) label: SharedString,
    pub(super) variant: PanelActionVariant,
    pub(super) disabled: bool,
    #[allow(clippy::type_complexity)]
    pub(super) on_click: Arc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
    pub(super) dropdown_items: std::rc::Rc<Vec<PanelDropdownItem>>,
}

impl PanelAction {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        variant: PanelActionVariant,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            variant,
            disabled: false,
            on_click: Arc::new(on_click),
            dropdown_items: std::rc::Rc::new(Vec::new()),
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Append a dropdown menu item. Having one or more items turns this
    /// action into a split-button: clicking the label runs `on_click`,
    /// clicking the caret opens a popup menu containing these items.
    pub fn with_dropdown_item(
        mut self,
        label: impl Into<SharedString>,
        on_select: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        std::rc::Rc::make_mut(&mut self.dropdown_items).push(PanelDropdownItem {
            label: label.into(),
            on_select: Arc::new(on_select),
        });
        self
    }
}

pub struct InputPanel {
    pub area: gpui::Entity<InputState>,
    layout: InputPanelLayout,
    /// `true` keeps the inner `Input::appearance(true)` chrome (bg /
    /// border / radius) — set via [`with_borderless(false)`] (the
    /// default). `with_borderless(true)` flips it off when the
    /// surrounding pane already paints its own border.
    appearance: bool,
    actions: Vec<PanelAction>,
    _area_sub: Subscription,
}

impl InputPanel {
    pub fn new(layout: InputPanelLayout, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let area = cx.new(|cx_state| InputState::new(window, cx_state).multi_line(true));
        let sub = cx.subscribe_in(&area, window, |_, _, ev: &InputEvent, _, cx| match ev {
            // Submit on the `secondary` flavour of PressEnter, which
            // `gpui_component::Input` emits for `Cmd+Enter` in
            // multi-line mode (plain Enter inserts a newline).
            InputEvent::PressEnter { secondary } if *secondary => {
                cx.emit(InputPanelEvent::Submit);
            }
            InputEvent::Change => cx.emit(InputPanelEvent::Changed),
            _ => {}
        });
        Self {
            area,
            layout,
            appearance: true,
            actions: Vec::new(),
            _area_sub: sub,
        }
    }

    pub fn with_placeholder(
        self,
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let s = placeholder.into();
        self.area
            .update(cx, |a, cx_state| a.set_placeholder(s, window, cx_state));
        self
    }

    /// No-op kept so existing call sites compile. The inner
    /// `gpui_component::Input` decides whether to draw a focus ring
    /// from its own appearance flag — `.appearance(true)` (default)
    /// paints the ring through `cx.theme().ring`, `.appearance(false)`
    /// (set by [`with_borderless`]) suppresses it. No per-panel signal
    /// is needed either way.
    pub fn with_focus_ring(self, _enabled: bool, _cx: &mut Context<Self>) -> Self {
        self
    }

    pub fn with_borderless(mut self, _cx: &mut Context<Self>) -> Self {
        self.appearance = false;
        self
    }

    pub fn with_action(mut self, action: PanelAction) -> Self {
        self.actions.push(action);
        self
    }

    pub fn set_action_disabled(&mut self, id: &str, disabled: bool, cx: &mut Context<Self>) {
        if let Some(a) = self.actions.iter_mut().find(|a| a.id.as_ref() == id)
            && a.disabled != disabled
        {
            a.disabled = disabled;
            cx.notify();
        }
    }

    /// Update the display label of an action button (e.g. after a locale change).
    pub fn set_action_label(
        &mut self,
        id: &str,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        if let Some(a) = self.actions.iter_mut().find(|a| a.id.as_ref() == id) {
            a.label = label.into();
            cx.notify();
        }
    }

    /// Update the label of the first dropdown item under an action button.
    pub fn set_action_dropdown_label(
        &mut self,
        action_id: &str,
        item_index: usize,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        if let Some(a) = self.actions.iter_mut().find(|a| a.id.as_ref() == action_id) {
            let items = std::rc::Rc::make_mut(&mut a.dropdown_items);
            if let Some(item) = items.get_mut(item_index) {
                item.label = label.into();
                cx.notify();
            }
        }
    }

    /// Returns an owned snapshot of the panel's text. The buffer lives
    /// in a sibling `Entity<InputState>`, so callers can't hold a
    /// `&str` through the entity guard and must take a clone.
    pub fn text(&self, cx: &App) -> String {
        self.area.read(cx).value().to_string()
    }

    pub fn is_empty(&self, cx: &App) -> bool {
        self.area.read(cx).value().is_empty()
    }

    pub fn clear(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.area
            .update(cx, |a, cx_state| a.set_value("", window, cx_state));
    }

    pub fn set_text(&self, value: impl Into<String>, window: &mut Window, cx: &mut Context<Self>) {
        let v = value.into();
        self.area
            .update(cx, |a, cx_state| a.set_value(v, window, cx_state));
    }

    // `copy` / `cut` / `paste` / `select_all` are deliberately omitted:
    // `gpui_component::Input` already registers its own
    // `on_action(Copy / Cut / Paste / SelectAll)` handlers — when the
    // focus is on the input these run automatically through the
    // `"Input"` key context. Surrounding panes used to call
    // `panel.copy/cut/...` to extend that handling to clicks landing
    // on the outer wrapper (drop target, padding); the trade-off is
    // that Cmd+C / Cmd+V now only act on the input while it has
    // keyboard focus. Drop / drag-and-drop flows continue to work
    // through `insert_at_cursor` below.

    pub fn insert_at_cursor(&self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let t = text.to_string();
        self.area
            .update(cx, |a, cx_state| a.insert(t, window, cx_state));
    }
}

impl Focusable for InputPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.area.focus_handle(cx)
    }
}

impl EventEmitter<InputPanelEvent> for InputPanel {}

impl Render for InputPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bar_border = theme::current(cx).git_commit_border;
        let area = Input::new(&self.area).small().appearance(self.appearance);

        let buttons: Vec<AnyElement> = self
            .actions
            .iter()
            .map(|action| {
                use crate::ui::Disableable as _;
                let handler = Arc::clone(&action.on_click);
                let btn = match action.variant {
                    PanelActionVariant::Primary => {
                        crate::ui::button_primary(action.id.clone(), action.label.clone())
                    }
                    PanelActionVariant::Secondary => {
                        crate::ui::button(action.id.clone(), action.label.clone())
                    }
                    PanelActionVariant::Danger => {
                        crate::ui::button_danger(action.id.clone(), action.label.clone())
                    }
                };
                let btn = btn
                    .disabled(action.disabled)
                    .on_click(move |ev, win, cx| handler(ev, win, cx));

                if action.dropdown_items.is_empty() {
                    btn.into_any_element()
                } else {
                    let items = std::rc::Rc::clone(&action.dropdown_items);
                    let dropdown =
                        DropdownButton::new(SharedString::from(format!("{}-split", action.id)))
                            .xsmall()
                            .button(btn)
                            .disabled(action.disabled);
                    let dropdown = match action.variant {
                        PanelActionVariant::Primary => dropdown.primary(),
                        PanelActionVariant::Secondary => dropdown,
                        PanelActionVariant::Danger => dropdown.danger(),
                    };
                    dropdown
                        .dropdown_menu(move |menu, _window, _cx| {
                            let mut m = menu;
                            for it in items.iter() {
                                let on_select = Arc::clone(&it.on_select);
                                m = m.item(
                                    PopupMenuItem::new(it.label.clone())
                                        .on_click(move |_, win, app_cx| on_select(win, app_cx)),
                                );
                            }
                            m
                        })
                        .into_any_element()
                }
            })
            .collect();

        match self.layout {
            InputPanelLayout::ActionsBelow => div()
                .flex()
                .flex_col()
                .w_full()
                .gap(px(theme::INPUT_PANEL_SECTION_GAP))
                .child(
                    div()
                        .flex_1()
                        .min_h(px(theme::INPUT_PANEL_MIN_H))
                        .child(area),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(theme::INPUT_PANEL_BUTTON_GAP))
                        .children(buttons),
                )
                .into_any_element(),
            InputPanelLayout::ActionsRight => div()
                .flex()
                .flex_row()
                .w_full()
                .h_full()
                .gap(px(theme::INPUT_PANEL_SECTION_GAP))
                .child(div().flex_1().h_full().child(area))
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(theme::INPUT_PANEL_BUTTON_GAP))
                        .children(buttons),
                )
                .into_any_element(),
            InputPanelLayout::ActionsFloating => div()
                .relative()
                .w_full()
                .h_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .w_full()
                        .flex_1()
                        .min_h(px(
                            theme::INPUT_PANEL_MIN_H + theme::INPUT_PANEL_FLOATING_BAR_H
                        ))
                        .pb(px(theme::INPUT_PANEL_FLOATING_BAR_H))
                        .child(area),
                )
                .child(
                    div()
                        .absolute()
                        .bottom(px(0.))
                        .left(px(0.))
                        .w_full()
                        .h(px(theme::INPUT_PANEL_FLOATING_BAR_H))
                        .border_t_1()
                        .border_color(bar_border)
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_end()
                        .px(px(theme::GIT_COMMIT_PAD))
                        .gap(px(theme::GIT_COMMIT_BUTTON_GAP))
                        .children(buttons),
                )
                .into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_action_disabled_builder() {
        let a = PanelAction::new("id", "Label", PanelActionVariant::Primary, |_, _, _| {})
            .disabled(true);
        assert!(a.disabled);
    }

    #[test]
    fn layout_variants_exist() {
        let _ = InputPanelLayout::ActionsBelow;
        let _ = InputPanelLayout::ActionsRight;
        let _ = InputPanelLayout::ActionsFloating;
    }
}
