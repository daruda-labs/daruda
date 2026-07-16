//! Stateless, interactive chevron disclosure primitive.
//!
//! The caller owns fold state; this widget only renders open/closed glyphs and
//! fires `on_toggle`. Colour is caller-supplied or inherited, avoiding a
//! hardcoded `hsla(..)` fallback under the inline-literal rule.

use crate::ui::theme;
use gpui::{
    App, ClickEvent, ElementId, Hsla, IntoElement, RenderOnce, Window, div, prelude::*, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

/// Boxed click handler alias to keep the field type readable.
type OnToggle = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Construct a disclosure chevron; the caller owns fold state.
pub fn disclosure(id: impl Into<ElementId>, is_open: bool) -> Disclosure {
    Disclosure::new(id, is_open)
}

/// Stateless chevron disclosure toggle.
#[derive(IntoElement)]
pub struct Disclosure {
    id: ElementId,
    is_open: bool,
    color: Option<Hsla>,
    size: Option<f32>,
    on_toggle: Option<OnToggle>,
}

impl Disclosure {
    /// Create a disclosure chevron for the caller-owned state.
    pub fn new(id: impl Into<ElementId>, is_open: bool) -> Self {
        Self {
            id: id.into(),
            is_open,
            color: None,
            size: None,
            on_toggle: None,
        }
    }

    /// Chevron color. Pass a `DarudaTheme` token (e.g. `t.text_subtle`).
    /// When omitted the glyph inherits the ambient `text_color`.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// Override the chevron pixel size. Defaults to `xsmall` when unset.
    pub fn size(mut self, size_px: f32) -> Self {
        self.size = Some(size_px);
        self
    }

    /// Click handler — fired when the chevron is clicked. The caller
    /// flips its own `is_open` state and re-renders.
    pub fn on_toggle(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Disclosure {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            is_open,
            color,
            size,
            on_toggle,
        } = self;

        let icon_name = if is_open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        let mut icon = Icon::new(icon_name).xsmall();
        if let Some(px_size) = size {
            icon = icon.with_size(Size::Size(px(px_size)));
        }
        if let Some(color) = color {
            icon = icon.text_color(color);
        }

        let el = div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .w(px(theme::DISCLOSURE_CHEVRON_W))
            .cursor_pointer()
            .child(icon);

        match on_toggle {
            Some(handler) => el.on_click(handler),
            None => el,
        }
    }
}
