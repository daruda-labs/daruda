//! Helpers shared by `CreateWorktreeModal` and `SessionHostModal` — both
//! render a registry-select field label and translate a
//! [`SessionHostError`] into its localized banner message, so the two
//! sibling modals show identical field chrome and identical wording for
//! the same rejected field.

use gpui::{IntoElement, SharedString, div, prelude::*, px};

use crate::lane::session_host::{SessionHostError, SessionHostField};
use crate::surface::strings as s;
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

/// Map a [`SessionHostError`] to its localized banner message.
pub(super) fn session_host_error_to_msg(e: SessionHostError) -> String {
    match e {
        SessionHostError::Empty(SessionHostField::Target) => s::session_host_err_target_empty(),
        SessionHostError::Empty(SessionHostField::Container) => {
            s::session_host_err_container_empty()
        }
        SessionHostError::Empty(SessionHostField::SessionPath) => {
            s::session_host_err_session_path_empty()
        }
        SessionHostError::Unsafe(SessionHostField::Target) => s::session_host_err_target_unsafe(),
        SessionHostError::Unsafe(SessionHostField::Container) => {
            s::session_host_err_container_unsafe()
        }
        SessionHostError::Unsafe(SessionHostField::SessionPath) => {
            s::session_host_err_session_path_unsafe()
        }
    }
}
