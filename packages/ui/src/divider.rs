//! Divider — re-export. No `Sizable` impl, no auto-apply.

// gpui-component 0.8 renamed its divider to `Separator`; callers keep the
// established name.
pub use gpui_component::separator::Separator as Divider;
