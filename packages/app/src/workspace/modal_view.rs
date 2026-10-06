use gpui::{Focusable, Render};

/// Marker trait for entities opened as a full-body form modal via
/// `dialog_helpers::open_form_modal`.
///
/// Enforces that only intentional modal entities are passed to
/// `open_form_modal`. Implementors must also satisfy `Render + Focusable`,
/// which are enforced by the `open_form_modal` generic bound.
pub trait ModalView: Render + Focusable {}
