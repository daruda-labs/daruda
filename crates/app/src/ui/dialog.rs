//! Dialog re-exports — Phase 4.a uses `window.open_dialog(...)` directly.
//!
//! Higher-level `confirm()` / `single_field()` / `form()` helpers will
//! land in Phase 4.b / 4.c. For now this module is the single import
//! site for dialog-construction types so call sites stay off
//! `gpui_component::*`.

pub use gpui_component::WindowExt;
pub use gpui_component::button::ButtonVariant;
pub use gpui_component::dialog::{Dialog, DialogButtonProps};
