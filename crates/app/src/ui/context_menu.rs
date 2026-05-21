//! Right-click / context menu. A single panel with a flat list of
//! clickable items, positioned at an absolute anchor (usually the
//! click position). The host owns the open/close state + the
//! currently-relevant item list; this widget is stateless render.
//!
//! Dismissal: click-outside is wired the same way as [`ModalLayer`]
//! — the caller wraps the menu in a transparent full-screen
//! backdrop whose click-handler sets open = None. Submenus are
//! intentionally not supported yet.
//!
//! Current consumer: none (W-8 lane row right-click will be the
//! first). Kept here so the W-8 feature commit doesn't also have
//! to invent the widget.

use std::rc::Rc;

use crate::ui::theme;
use gpui::{
    App, ElementId, IntoElement, MouseButton, MouseDownEvent, Pixels, Point, RenderOnce,
    SharedString, Window, div, prelude::*, px,
};

#[derive(Clone)]
pub enum ContextMenuItem {
    Item {
        label: SharedString,
        disabled: bool,
        /// Hover tooltip — shown even on disabled items so the user
        /// understands why the action is unavailable.
        tooltip: Option<SharedString>,
        #[allow(clippy::type_complexity)]
        on_click: Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>,
    },
    Separator,
}

impl ContextMenuItem {
    pub fn new(
        label: impl Into<SharedString>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self::Item {
            label: label.into(),
            disabled: false,
            tooltip: None,
            on_click: Rc::new(on_click),
        }
    }

    pub fn separator() -> Self {
        Self::Separator
    }

    pub fn disabled(self, disabled: bool) -> Self {
        match self {
            Self::Item {
                label,
                on_click,
                tooltip,
                ..
            } => Self::Item {
                label,
                disabled,
                tooltip,
                on_click,
            },
            Self::Separator => Self::Separator,
        }
    }

    /// Attach a tooltip that appears on hover.  Useful for disabled items
    /// where the user needs to know why the action is unavailable.
    pub fn with_tooltip(self, text: impl Into<SharedString>) -> Self {
        match self {
            Self::Item {
                label,
                disabled,
                on_click,
                ..
            } => Self::Item {
                label,
                disabled,
                tooltip: Some(text.into()),
                on_click,
            },
            Self::Separator => Self::Separator,
        }
    }
}

/// Which corner of the menu lands on the supplied `position`. The
/// default `TopLeft` matches the standard right-click idiom (menu
/// expands down-right from the click). `BottomRight` is the inverse
/// for chips anchored near the right edge of a tab bar — the menu
/// expands up-left so it stays inside the workspace bounds.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum ContextMenuCorner {
    #[default]
    TopLeft,
    BottomRight,
}

#[derive(IntoElement)]
pub struct ContextMenu {
    id: ElementId,
    position: Point<Pixels>,
    items: Vec<ContextMenuItem>,
    corner: ContextMenuCorner,
    /// Size of the parent container the menu is rendered inside.
    /// Required to translate `BottomRight` anchoring into a
    /// parent-relative `right`/`bottom` inset. Ignored for `TopLeft`.
    parent_size: gpui::Size<Pixels>,
}

impl ContextMenu {
    pub fn new(id: impl Into<ElementId>, position: Point<Pixels>) -> Self {
        Self {
            id: id.into(),
            position,
            items: Vec::new(),
            corner: ContextMenuCorner::TopLeft,
            parent_size: gpui::size(px(0.0), px(0.0)),
        }
    }

    pub fn item(mut self, item: ContextMenuItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn items(mut self, items: impl IntoIterator<Item = ContextMenuItem>) -> Self {
        self.items.extend(items);
        self
    }

    /// Anchor the menu to a non-default corner. `parent_size` must
    /// match the bounds of the container the menu is mounted inside
    /// (workspace backdrop = viewport size in daruda) so the inverted
    /// `right` / `bottom` inset lands the chosen corner on `position`.
    pub fn anchor(mut self, corner: ContextMenuCorner, parent_size: gpui::Size<Pixels>) -> Self {
        self.corner = corner;
        self.parent_size = parent_size;
        self
    }
}

impl RenderOnce for ContextMenu {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            position,
            items,
            corner,
            parent_size,
        } = self;

        let t = theme::current(cx);
        let panel_border = t.modal_panel_border;
        let panel_bg = t.modal_panel_bg;
        let item_text = t.modal_text_primary;
        let item_disabled_text = t.disabled_item_text;
        let item_hover_bg = t.lane_row_hover_bg;

        let mut menu = div()
            .id(id)
            .absolute()
            .map(|m| match corner {
                ContextMenuCorner::TopLeft => m.left(position.x).top(position.y),
                ContextMenuCorner::BottomRight => m
                    .right(parent_size.width - position.x)
                    .bottom(parent_size.height - position.y),
            })
            // Panel visuals reuse the modal input chrome so the menu
            // reads as the same "surface" the rest of the UI uses.
            .flex()
            .flex_col()
            .rounded(px(theme::MODAL_BUTTON_RADIUS))
            .border_1()
            .border_color(panel_border)
            .bg(panel_bg)
            .py(px(theme::MODAL_FOOTER_MARGIN_TOP))
            // Catch clicks so they don't bubble to a parent backdrop
            // that might be trying to dismiss the menu.
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
            });

        for (idx, item) in items.into_iter().enumerate() {
            match item {
                ContextMenuItem::Separator => {
                    let sep = div()
                        .id(("context-menu-sep", idx))
                        .h(px(theme::CONTEXT_MENU_SEPARATOR_H))
                        .mx(px(theme::MODAL_BUTTON_PAD_X))
                        .my(px(theme::MODAL_FOOTER_MARGIN_TOP))
                        .bg(panel_border);
                    menu = menu.child(sep);
                }
                ContextMenuItem::Item {
                    label,
                    disabled,
                    tooltip,
                    on_click,
                } => {
                    let row = div()
                        .id(("context-menu-item", idx))
                        .px(px(theme::MODAL_BUTTON_PAD_X))
                        .py(px(theme::MODAL_BUTTON_PAD_Y))
                        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                        .text_color(if disabled {
                            item_disabled_text
                        } else {
                            item_text
                        })
                        .when(!disabled, |d| {
                            d.cursor_pointer()
                                .hover(move |d| d.bg(item_hover_bg))
                                .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
                                    (on_click)(ev, window, cx);
                                })
                        })
                        .when_some(tooltip, |d, msg| d.tooltip(crate::ui::tooltip::text(msg)))
                        .child(label);
                    menu = menu.child(row);
                }
            }
        }

        menu
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render, point};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn anchor() -> Point<Pixels> {
        point(px(0.0), px(0.0))
    }

    fn noop_click() -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        |_, _, _| {}
    }

    // ---- Item construction ----

    #[test]
    fn item_defaults_to_enabled() {
        if let ContextMenuItem::Item {
            label, disabled, ..
        } = ContextMenuItem::new("copy", noop_click())
        {
            assert_eq!(label.as_ref(), "copy");
            assert!(!disabled);
        } else {
            panic!("expected Item variant");
        }
    }

    #[test]
    fn item_disabled_builder_sets_flag() {
        let item = ContextMenuItem::new("delete", noop_click()).disabled(true);
        if let ContextMenuItem::Item { disabled, .. } = item {
            assert!(disabled);
        } else {
            panic!("expected Item variant");
        }
    }

    #[test]
    fn item_disabled_can_be_unset() {
        // Builder chain should be re-settable — caller might toggle
        // disabled based on runtime state.
        let item = ContextMenuItem::new("x", noop_click())
            .disabled(true)
            .disabled(false);
        if let ContextMenuItem::Item { disabled, .. } = item {
            assert!(!disabled);
        } else {
            panic!("expected Item variant");
        }
    }

    #[test]
    fn item_with_tooltip_stores_text() {
        let item = ContextMenuItem::new("merge", noop_click()).with_tooltip("Commit first");
        if let ContextMenuItem::Item { tooltip, .. } = item {
            assert_eq!(tooltip.as_ref().map(|s| s.as_ref()), Some("Commit first"));
        } else {
            panic!("expected Item variant");
        }
    }

    #[test]
    fn item_tooltip_preserved_through_disabled() {
        // .disabled() must not clear an existing tooltip.
        let item = ContextMenuItem::new("x", noop_click())
            .with_tooltip("reason")
            .disabled(true);
        if let ContextMenuItem::Item {
            tooltip, disabled, ..
        } = item
        {
            assert!(disabled);
            assert_eq!(tooltip.as_ref().map(|s| s.as_ref()), Some("reason"));
        } else {
            panic!("expected Item variant");
        }
    }

    #[test]
    fn item_no_tooltip_by_default() {
        let item = ContextMenuItem::new("a", noop_click());
        if let ContextMenuItem::Item { tooltip, .. } = item {
            assert!(tooltip.is_none());
        } else {
            panic!("expected Item variant");
        }
    }

    #[test]
    fn separator_is_separator() {
        assert!(matches!(
            ContextMenuItem::separator(),
            ContextMenuItem::Separator
        ));
    }

    // ---- ContextMenu builder ----

    #[test]
    fn new_starts_empty() {
        let menu = ContextMenu::new("id", anchor());
        assert_eq!(menu.items.len(), 0);
    }

    #[test]
    fn item_builder_appends_one() {
        let menu = ContextMenu::new("id", anchor()).item(ContextMenuItem::new("a", noop_click()));
        assert_eq!(menu.items.len(), 1);
        if let ContextMenuItem::Item { label, .. } = &menu.items[0] {
            assert_eq!(label.as_ref(), "a");
        } else {
            panic!("expected Item variant");
        }
    }

    #[test]
    fn items_builder_extends_list() {
        let menu = ContextMenu::new("id", anchor()).items(vec![
            ContextMenuItem::new("a", noop_click()),
            ContextMenuItem::new("b", noop_click()),
            ContextMenuItem::new("c", noop_click()),
        ]);
        assert_eq!(menu.items.len(), 3);
        let labels: Vec<_> = menu
            .items
            .iter()
            .filter_map(|i| {
                if let ContextMenuItem::Item { label, .. } = i {
                    Some(label.as_ref())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(labels, vec!["a", "b", "c"]);
    }

    #[test]
    fn item_and_items_mix_preserves_order() {
        // Order matters: the right-click menu renders top-to-bottom in
        // insertion order, so .item() then .items() must append after.
        let menu = ContextMenu::new("id", anchor())
            .item(ContextMenuItem::new("one", noop_click()))
            .items(vec![
                ContextMenuItem::new("two", noop_click()),
                ContextMenuItem::new("three", noop_click()),
            ])
            .item(ContextMenuItem::new("four", noop_click()));
        let labels: Vec<_> = menu
            .items
            .iter()
            .filter_map(|i| {
                if let ContextMenuItem::Item { label, .. } = i {
                    Some(label.as_ref())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(labels, vec!["one", "two", "three", "four"]);
    }

    #[test]
    fn position_is_stored_verbatim() {
        // ContextMenu anchors at the click coordinate; the caller
        // passes the raw position and expects no adjustment.
        let menu = ContextMenu::new("id", point(px(42.0), px(99.0)));
        assert_eq!(menu.position.x, px(42.0));
        assert_eq!(menu.position.y, px(99.0));
    }

    // Minimal `Render` stand-in so `cx.add_window` has a view it can
    // host while the test invokes the on_click closure.
    struct Host;
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    fn fake_click() -> MouseDownEvent {
        MouseDownEvent {
            button: MouseButton::Left,
            position: point(px(0.0), px(0.0)),
            modifiers: gpui::Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }
    }

    #[gpui::test]
    fn on_click_closure_fires_when_invoked(cx: &mut gpui::TestAppContext) {
        // The render path passes the stored Rc<Fn> to
        // `div().on_mouse_down(MouseButton::Left, move |ev, window, cx| …)`.
        // If a future change accidentally drops or swaps the closure,
        // this test catches it without needing a live click flow.
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let hits_clone = hits.clone();
        let item = ContextMenuItem::new("ping", move |_, _, _| {
            hits_clone.fetch_add(1, Ordering::SeqCst);
        });
        let wh = cx.add_window(|_, _| Host);
        cx.update_window(wh.into(), |_, window, cx| {
            if let ContextMenuItem::Item { on_click, .. } = &item {
                (on_click)(&fake_click(), window, cx);
                (on_click)(&fake_click(), window, cx);
            }
        })
        .unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }
}
