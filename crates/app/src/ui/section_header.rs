//! Reusable section-header row.
//!
//! Pulled out of three near-identical chains in
//! `workspace/sidebar/{worktrees/list, git_changes/mod, files/mod}.rs`.
//! Each rendered the same shape — `flex_row + items_center +
//! justify_between + theme::WORKTREE_SECTION_HEADER_FONT_SIZE +
//! DOCK_HEADER_TEXT + label + optional actions` — only differing in
//! padding (some sites embed the header inside an outer column whose
//! padding is handled separately) and whether the label needs
//! truncation.
//!
//! Padding is opt-in via [`SectionHeader::padding`] / [`pad_x`] /
//! [`pad_y`] so callers that already wrap the header in a parent column
//! don't double-pad. Label truncation is also opt-in because the file
//! tree / git-changes headers can host long branch names.
//!
//! ```ignore
//! use crate::ui::SectionHeader;
//!
//! SectionHeader::new("Worktrees")
//!     .padding(theme::WORKTREE_ROW_PAD_X, theme::WORKTREE_SECTION_PAD_Y)
//!     .actions(add_button)
//!
//! SectionHeader::new(format!("Git Changes — {branch}"))
//!     .truncate_label(true)
//!     .actions(remote_button_row)
//! ```

use crate::ui::theme;
use gpui::{AnyElement, App, IntoElement, RenderOnce, SharedString, Window, div, prelude::*, px};

/// Header row with a left label and optional right-aligned actions.
#[derive(IntoElement)]
pub struct SectionHeader {
    label: SharedString,
    actions: Option<AnyElement>,
    pad_x: Option<f32>,
    pad_y: Option<f32>,
    truncate_label: bool,
}

impl SectionHeader {
    /// Start a header with the given label. Padding and actions are
    /// off by default — opt in with [`padding`](Self::padding) /
    /// [`actions`](Self::actions).
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            actions: None,
            pad_x: None,
            pad_y: None,
            truncate_label: false,
        }
    }

    /// Right-aligned action slot — typically a button or a row of
    /// buttons. Pass any `IntoElement`.
    pub fn actions(mut self, el: impl IntoElement) -> Self {
        self.actions = Some(el.into_any_element());
        self
    }

    /// Set both horizontal and vertical padding in one call.
    pub fn padding(mut self, x: f32, y: f32) -> Self {
        self.pad_x = Some(x);
        self.pad_y = Some(y);
        self
    }

    /// Set horizontal padding only.
    pub fn pad_x(mut self, x: f32) -> Self {
        self.pad_x = Some(x);
        self
    }

    /// Set vertical padding only.
    pub fn pad_y(mut self, y: f32) -> Self {
        self.pad_y = Some(y);
        self
    }

    /// Wrap the label in `overflow_hidden + whitespace_nowrap` so a
    /// long string clips rather than reflowing the row.
    pub fn truncate_label(mut self, b: bool) -> Self {
        self.truncate_label = b;
        self
    }
}

impl RenderOnce for SectionHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            label,
            actions,
            pad_x,
            pad_y,
            truncate_label,
        } = self;

        let label_node = if truncate_label {
            div()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(label)
                .into_any_element()
        } else {
            div().child(label).into_any_element()
        };

        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .text_size(px(theme::WORKTREE_SECTION_HEADER_FONT_SIZE))
            .text_color(theme::current(cx).dock_header_text);

        if let Some(x) = pad_x {
            row = row.px(px(x));
        }
        if let Some(y) = pad_y {
            row = row.py(px(y));
        }

        row = row.child(label_node);
        if let Some(actions) = actions {
            row = row.child(actions);
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_no_padding_or_actions() {
        let h = SectionHeader::new("Title");
        assert_eq!(h.label.as_ref(), "Title");
        assert!(h.actions.is_none());
        assert!(h.pad_x.is_none());
        assert!(h.pad_y.is_none());
        assert!(!h.truncate_label);
    }

    #[test]
    fn actions_sets_slot() {
        let h = SectionHeader::new("Title").actions(div().child("[+]"));
        assert!(h.actions.is_some());
    }

    #[test]
    fn padding_sets_both_axes() {
        let h = SectionHeader::new("Title").padding(8.0, 4.0);
        assert_eq!(h.pad_x, Some(8.0));
        assert_eq!(h.pad_y, Some(4.0));
    }

    #[test]
    fn pad_x_and_pad_y_independent() {
        let h = SectionHeader::new("Title").pad_x(8.0);
        assert_eq!(h.pad_x, Some(8.0));
        assert!(h.pad_y.is_none());

        let h = SectionHeader::new("Title").pad_y(4.0);
        assert!(h.pad_x.is_none());
        assert_eq!(h.pad_y, Some(4.0));
    }

    #[test]
    fn truncate_label_flag_propagates() {
        let h = SectionHeader::new("Title").truncate_label(true);
        assert!(h.truncate_label);
    }

    #[test]
    fn label_accepts_owned_string() {
        let owned = String::from("dynamic");
        let h = SectionHeader::new(owned);
        assert_eq!(h.label.as_ref(), "dynamic");
    }

    #[test]
    fn full_chain_builds() {
        let h = SectionHeader::new("Title")
            .padding(8.0, 4.0)
            .truncate_label(true)
            .actions(div().child("button"));
        assert!(h.actions.is_some());
        assert_eq!(h.pad_x, Some(8.0));
        assert_eq!(h.pad_y, Some(4.0));
        assert!(h.truncate_label);
    }
}
