//! Compact data badge over `gpui_component::Tag::custom`.
//!
//! The wrapper applies daruda badge metrics/palette while keeping a small
//! builder API for optional monospace text, explicit colours, and opt-in
//! truncation. It has no icon or interactivity; wrap it in a parent for clicks.

use crate::ui::theme;
use gpui::{App, Hsla, IntoElement, RenderOnce, SharedString, Window, prelude::*, px};
use gpui_component::Sizable as _;
use gpui_component::tag::Tag;

/// Stateless badge widget. Optional colours defer to the live theme unless
/// the caller overrides them.
#[derive(IntoElement)]
pub struct Badge {
    label: SharedString,
    bg_color: Option<Hsla>,
    border_color: Option<Hsla>,
    text_color: Option<Hsla>,
    monospace: bool,
    truncate: bool,
}

impl Badge {
    /// Create a badge with theme-derived default colours.
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            bg_color: None,
            border_color: None,
            text_color: None,
            monospace: false,
            truncate: false,
        }
    }

    /// Override the fill color.
    pub fn bg_color(mut self, color: Hsla) -> Self {
        self.bg_color = Some(color);
        self
    }

    /// Override the 1px border color. Pass the same value as
    /// [`Self::bg_color`] (or with reduced alpha) to render the badge
    /// as a flat tint without a visible outline.
    pub fn border_color(mut self, color: Hsla) -> Self {
        self.border_color = Some(color);
        self
    }

    /// Override the text color.
    pub fn text_color(mut self, color: Hsla) -> Self {
        self.text_color = Some(color);
        self
    }

    /// Render the label in monospace. Recommended for hash-like
    /// identifiers (session ids, commit hashes) so identical-looking
    /// glyphs (`I`/`l`/`1`, `O`/`0`) line up across rows.
    pub fn monospace(mut self) -> Self {
        self.monospace = true;
        self
    }

    /// Allow the badge to shrink and truncate its label when its
    /// parent is space-constrained. Default is to size to content.
    pub fn truncate(mut self) -> Self {
        self.truncate = true;
        self
    }
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            label,
            bg_color,
            border_color,
            text_color,
            monospace,
            truncate,
        } = self;

        let t = theme::current(cx);
        let bg_color = bg_color.unwrap_or(t.badge_bg);
        let border_color = border_color.unwrap_or(t.border);
        let text_color = text_color.unwrap_or(t.text_body);

        let mut tag = Tag::custom(bg_color, text_color, border_color)
            .small()
            .rounded(px(theme::BADGE_RADIUS))
            .px(px(theme::BADGE_PAD_X))
            .py(px(theme::BADGE_PAD_Y))
            .text_size(px(theme::BADGE_FONT_SIZE))
            .child(label);

        if monospace {
            tag = tag.font_family(theme::FONT_FAMILY_MONOSPACE);
        }
        if truncate {
            tag = tag.min_w_0().overflow_hidden();
        } else {
            tag = tag.flex_none();
        }
        tag
    }
}
