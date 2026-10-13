//! Skills-specific render helpers. Each modal owns its own input
//! subscriptions and footer inline now that the `form_modal` trait
//! plumbing has been retired in favour of GPUI's tab system +
//! Dialog's built-in Confirm / Cancel actions.

use crate::ui::theme;
use gpui::{IntoElement, SharedString, div, prelude::*, px};

/// Small label rendered above a form field. Matches the typography of
/// `field_row` from `ui::form_helpers` but lives here because the
/// Skills modal stacks label-on-top instead of inline.
pub(super) fn field_label(
    text: impl Into<SharedString>,
    t: &crate::ui::theme::DarudaTheme,
) -> impl IntoElement {
    div()
        .text_size(px(theme::FONT_SIZE_LG))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(t.text_body)
        .child(text.into())
}
