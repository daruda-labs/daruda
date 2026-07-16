//! Reusable left-dock section-header row.
//!
//! Padding and label truncation are opt-in because some callers already own
//! outer padding, and file/git headers can contain long branch names.

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
    /// Start a header with padding/actions off by default.
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            actions: None,
            pad_x: None,
            pad_y: None,
            truncate_label: false,
        }
    }

    /// Right-aligned action slot.
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
            .text_size(px(theme::LANE_SECTION_HEADER_FONT_SIZE))
            .text_color(theme::current(cx).text_muted);

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
