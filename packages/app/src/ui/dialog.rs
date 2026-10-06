//! Dialog re-exports for `window.open_dialog(...)` call sites. The single
//! import site for dialog-construction types so callers stay off
//! `gpui_component::*` directly. Higher-level open helpers live in
//! `crate::workspace::dialog_helpers`.

pub use gpui_component::WindowExt;
pub use gpui_component::button::ButtonVariant;
pub use gpui_component::dialog::{Dialog, DialogButtonProps};
