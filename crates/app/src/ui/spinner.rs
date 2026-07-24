//! Loading-spinner factory over `gpui_component::spinner::Spinner`.
//!
//! `Spinner` is `Sizable`; the factory applies `xsmall()` so it drops
//! cleanly into compact chrome (status-bar dropdown rows, inline
//! in-progress indicators) without a call-site size chain.

use gpui_component::Sizable as _;

pub use gpui_component::spinner::Spinner;

/// A small cycling loading spinner. Chain `.color(hsla)` to override
/// the icon's default theme color.
pub fn spinner() -> Spinner {
    Spinner::new().xsmall()
}
