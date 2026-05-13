//! Compact pill-shaped label, wrapper over `gpui_component::Tag::custom`.
//!
//! Builds a `Tag` with `TagVariant::Custom` carrying daruda's badge
//! palette and overrides the size-driven padding/font with daruda's
//! metric constants. The public builder shape (`Badge::new(label)
//! .monospace().bg_color(c)...`) is kept verbatim so the existing
//! call sites (right_panel/usage.rs, right_panel/tasks.rs) compile
//! unchanged.
//!
//! Use cases (current + planned):
//! - Right-panel Usage tab: short session id `[abc12345]` next to
//!   each session row (R-3).
//! - Right-panel Tasks tab: per-session status badge `[abc12345]`
//!   tinted by Claude session status (R-12).
//! - Future: short commit-hash next to a worktree row, build-id
//!   next to a Tasks log entry, etc.
//!
//! ## Usage
//! ```ignore
//! use crate::ui::Badge;
//!
//! parent.child(Badge::new(&session_id[..8]).monospace())
//!
//! parent.child(
//!     Badge::new(&session_id[..8])
//!         .monospace()
//!         .text_color(theme::SOMETHING_RED)
//!         .border_color(theme::SOMETHING_RED.alpha(0.4))
//! )
//! ```
//!
//! ## Design notes
//! - **No icon slot.** zed's `Chip` carries an optional `IconName`;
//!   daruda uses `ui::status_indicator` next to a `Badge` instead.
//! - **No interactivity.** No `on_click`, no tooltip — `Badge` is
//!   pure data display. Wrap in a clickable parent if needed.
//! - **`truncate` is opt-in.** Default behaviour lets the badge size
//!   to its content; callers in width-constrained rows ask for
//!   truncation explicitly.

use crate::ui::theme;
use gpui::{App, Hsla, IntoElement, RenderOnce, SharedString, Window, prelude::*, px};
use gpui_component::Sizable as _;
use gpui_component::tag::Tag;

/// Stateless badge widget. Builder-style; see module docs for usage.
///
/// `bg_color` / `border_color` / `text_color` are `Option<Hsla>` so
/// the default path defers to the live `DarudaTheme` Global at paint
/// time (light-mode flip works for free) and explicit overrides win
/// at the call site.
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
    /// Create a badge with the given label. Default colours fall back
    /// to `DarudaTheme::badge_bg` / `badge_border` / `badge_text` at
    /// paint time so theme switches re-tone the badge.
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
        let border_color = border_color.unwrap_or(t.badge_border);
        let text_color = text_color.unwrap_or(t.badge_text);

        let mut tag = Tag::custom(bg_color, text_color, border_color)
            .xsmall()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_defer_to_theme() {
        let b = Badge::new("abc12345");
        assert_eq!(b.bg_color, None);
        assert_eq!(b.border_color, None);
        assert_eq!(b.text_color, None);
        assert!(!b.monospace);
        assert!(!b.truncate);
    }

    #[test]
    fn label_round_trips_through_shared_string() {
        let b = Badge::new("abc12345");
        assert_eq!(b.label.as_ref(), "abc12345");
        let owned = String::from("def67890");
        let b2 = Badge::new(owned);
        assert_eq!(b2.label.as_ref(), "def67890");
    }

    #[test]
    fn color_modifiers_override_each_independently() {
        let b = Badge::new("x")
            .bg_color(theme::DOCK_BG)
            .border_color(theme::DOCK_BORDER)
            .text_color(theme::MUTED_TEXT);
        assert_eq!(b.bg_color, Some(theme::DOCK_BG));
        assert_eq!(b.border_color, Some(theme::DOCK_BORDER));
        assert_eq!(b.text_color, Some(theme::MUTED_TEXT));
    }

    #[test]
    fn monospace_modifier_flips_flag() {
        let b = Badge::new("x").monospace();
        assert!(b.monospace);
    }

    #[test]
    fn truncate_modifier_flips_flag() {
        let b = Badge::new("x").truncate();
        assert!(b.truncate);
    }

    #[test]
    fn modifier_chain_is_idempotent() {
        let b = Badge::new("x")
            .text_color(theme::FAINT_TEXT)
            .text_color(theme::MUTED_TEXT);
        assert_eq!(b.text_color, Some(theme::MUTED_TEXT));
    }
}
