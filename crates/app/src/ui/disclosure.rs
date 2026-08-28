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

/// Which pair of glyphs a disclosure flips between.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum DisclosureAxis {
    /// `▶ / ▼` — the tree default: a closed node points at the content it holds.
    #[default]
    Horizontal,
    /// `▼ / ▲` — for a control that is a *boundary* rather than a tree node, so
    /// it cannot be mistaken for one of the rows it sits among: closed invites
    /// opening downward, open invites folding back up.
    Vertical,
}

/// The glyph a disclosure shows. Pulled out of `render` so the mapping is
/// assertable: a `Vertical` disclosure showing `ChevronUp` while closed inverts
/// the affordance, and nothing about the render path would say so.
fn chevron_icon(axis: DisclosureAxis, is_open: bool) -> IconName {
    match (axis, is_open) {
        (DisclosureAxis::Horizontal, false) => IconName::ChevronRight,
        (DisclosureAxis::Horizontal, true) => IconName::ChevronDown,
        (DisclosureAxis::Vertical, false) => IconName::ChevronDown,
        (DisclosureAxis::Vertical, true) => IconName::ChevronUp,
    }
}

/// Construct a disclosure chevron; the caller owns fold state.
pub fn disclosure(id: impl Into<ElementId>, is_open: bool) -> Disclosure {
    Disclosure::new(id, is_open)
}

/// Stateless chevron disclosure toggle.
#[derive(IntoElement)]
pub struct Disclosure {
    id: ElementId,
    is_open: bool,
    axis: DisclosureAxis,
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
            axis: DisclosureAxis::default(),
            color: None,
            size: None,
            on_toggle: None,
        }
    }

    /// Which glyph pair to flip between. See [`DisclosureAxis`].
    pub fn axis(mut self, axis: DisclosureAxis) -> Self {
        self.axis = axis;
        self
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
            axis,
            color,
            size,
            on_toggle,
        } = self;

        let mut icon = Icon::new(chevron_icon(axis, is_open)).xsmall();
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_component::IconNamed as _;

    /// `IconName` is a vendored enum with no `PartialEq`, so compare the asset
    /// each variant resolves to rather than patching the vendored tree.
    fn glyph(axis: DisclosureAxis, is_open: bool) -> String {
        chevron_icon(axis, is_open).path().to_string()
    }

    /// Both axes flip, and they disagree on the *closed* glyph — which is the
    /// whole point: a boundary control has to be distinguishable from the tree
    /// nodes it sits among.
    #[test]
    fn each_axis_flips_and_the_two_differ_when_closed() {
        for axis in [DisclosureAxis::Horizontal, DisclosureAxis::Vertical] {
            assert_ne!(
                glyph(axis, false),
                glyph(axis, true),
                "a disclosure showing one glyph in both states says nothing"
            );
        }
        assert_ne!(
            glyph(DisclosureAxis::Horizontal, false),
            glyph(DisclosureAxis::Vertical, false)
        );
    }

    /// `Vertical` closed must invite opening *downward*; the inverted pair reads
    /// as "already open".
    #[test]
    fn the_vertical_axis_points_down_while_closed() {
        assert!(glyph(DisclosureAxis::Vertical, false).ends_with("chevron-down.svg"));
        assert!(glyph(DisclosureAxis::Vertical, true).ends_with("chevron-up.svg"));
    }
}
