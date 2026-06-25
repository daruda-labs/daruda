//! Disclosure — a stateless chevron toggle primitive.
//!
//! daruda's equivalent of zed's `Disclosure`: a single clickable
//! chevron glyph that points **down** when open and **right** when
//! closed (matching the existing file-tree idiom in
//! `left_dock/files/mod.rs`). It is **stateless** — `is_open` is owned
//! by the caller; the primitive never stores fold state. Clicking
//! fires `on_toggle`; the caller flips its own state and re-renders.
//!
//! Unlike the file tree's `chevron_element` (a *static* glyph where the
//! enclosing row owns the click), this is *interactive*: it carries its
//! own stable `id` + click handler + pointer cursor.
//!
//! ## Usage
//! ```ignore
//! use crate::ui::disclosure;
//!
//! let t = crate::ui::theme::current(cx);
//! parent.child(
//!     disclosure(("block", block_ix), is_open)
//!         .color(t.text_subtle)
//!         .on_toggle(cx.listener(|this, _, _, cx| {
//!             this.toggle_block(block_ix, cx);
//!         })),
//! )
//! ```
//!
//! ## Design notes
//! - **Color is caller-supplied.** Pass a `DarudaTheme` token via
//!   [`Disclosure::color`] (e.g. `t.text_subtle`). When omitted the
//!   chevron inherits the surrounding `text_color` — no hardcoded
//!   fallback (the inline-literal ban forbids a `hsla(..)` default
//!   here).
//! - **Size is opt-in.** Default chevron size is `xsmall` (the daruda
//!   default for chrome glyphs). Pass an explicit pixel size via
//!   [`Disclosure::size`] only when a tighter / looser fit is needed.

use crate::ui::theme;
use gpui::{
    App, ClickEvent, ElementId, Hsla, IntoElement, RenderOnce, Window, div, prelude::*, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

/// Boxed click handler stored by [`Disclosure`]. Aliased so the field type
/// stays readable (and clears clippy's `type_complexity` lint), mirroring how
/// zed types its own `Disclosure` handler.
type OnToggle = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Construct a stateless disclosure chevron. `is_open` selects the
/// glyph (`ChevronDown` when open, `ChevronRight` when closed); the
/// caller owns the fold state. Chain [`Disclosure::color`] /
/// [`Disclosure::on_toggle`] / [`Disclosure::size`].
pub fn disclosure(id: impl Into<ElementId>, is_open: bool) -> Disclosure {
    Disclosure::new(id, is_open)
}

/// Stateless chevron disclosure toggle. Builder-style; see module docs.
#[derive(IntoElement)]
pub struct Disclosure {
    id: ElementId,
    is_open: bool,
    color: Option<Hsla>,
    size: Option<f32>,
    on_toggle: Option<OnToggle>,
}

impl Disclosure {
    /// Create a disclosure chevron. `is_open` selects the glyph
    /// (`ChevronDown` when open, `ChevronRight` when closed) and is
    /// owned by the caller — this primitive holds no fold state.
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
