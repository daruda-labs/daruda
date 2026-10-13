use gpui::{Focusable, Render};

/// Marker trait for entities opened as a full-body form modal via
/// `dialog_helpers::open_form_modal`.
///
/// Enforces that only intentional modal entities are passed to
/// `open_form_modal`. Implementors must also satisfy `Render + Focusable`,
/// which are enforced by the `open_form_modal` generic bound.
pub trait ModalView: Render + Focusable {
    /// Opt-in height bound for forms with expandable or repeatable fields.
    fn fit_viewport(&self) -> bool {
        false
    }

    /// Actions outside the scrolling body of a viewport-fitted form.
    fn render_footer(&mut self, _cx: &mut gpui::Context<Self>) -> Vec<gpui::AnyElement>
    where
        Self: Sized,
    {
        Vec::new()
    }

    /// Opt-in close interception; existing modal lifecycles keep their policy.
    fn guards_close(&self) -> bool {
        false
    }

    fn request_close(&mut self, window: &mut gpui::Window, cx: &mut gpui::Context<Self>)
    where
        Self: Sized,
    {
        use crate::ui::WindowExt as _;
        window.close_dialog(cx);
    }
}
