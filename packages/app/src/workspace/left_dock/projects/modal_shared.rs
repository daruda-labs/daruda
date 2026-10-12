//! Helpers shared by `CreateWorktreeModal` and `SessionHostModal`, so the two
//! sibling modals show identical field chrome.

use gpui::{IntoElement, SharedString, div, prelude::*, px};

use crate::ui::theme;

/// Small label rendered above a form field — stack layout (label-on-top),
/// matching `right_dock::tools::modal_shared::field_label`'s typography.
pub(super) fn field_label(
    text: impl Into<SharedString>,
    t: &theme::DarudaTheme,
) -> impl IntoElement {
    div()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(t.text_muted)
        .child(text.into())
}
