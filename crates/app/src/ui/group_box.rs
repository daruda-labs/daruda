//! GroupBox wrapper over `gpui_component::group_box`.
//!
//! A styled container with an optional title. Variants: `.fill()`
//! (background only), `.outline()` (border only), `.normal()` (neither).
//! All padded variants use `theme.radius` corners and `p_4` content
//! padding. Caller chains `.outline()` / `.title(...)` / `.child(...)`.

pub use gpui_component::group_box::{GroupBox, GroupBoxVariants};

/// A new GroupBox (default `normal` variant). Chain a variant
/// (`.outline()` / `.fill()`) and `.child(...)` content.
pub fn group_box() -> GroupBox {
    GroupBox::new()
}
