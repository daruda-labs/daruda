//! `crate::ui::alert` — wrapper over `gpui_component::alert::Alert`.
//!
//! Daruda's old `ValidationBanner` had a `(severity, message)`-only API.
//! `gpui_component::Alert` requires an `ElementId` so two banners on
//! screen at once stay distinguishable to GPUI's stateful element
//! system. The factories take an explicit id; pick something stable
//! per call site (`"task-create-error"`, `"merge-conflict"`, ...).
//!
//! Sizing follows the project default — `xsmall()` is auto-applied
//! before any other modifier (CLAUDE.md §10).

use gpui::ElementId;
use gpui_component::Sizable as _;
use gpui_component::alert::Alert;
use gpui_component::text::Text;

pub use gpui_component::alert::AlertVariant;

/// Inline error banner. Use inside a modal body when validation fails.
pub fn error(id: impl Into<ElementId>, message: impl Into<Text>) -> Alert {
    Alert::error(id, message).small().banner()
}

/// Inline warning banner.
pub fn warning(id: impl Into<ElementId>, message: impl Into<Text>) -> Alert {
    Alert::warning(id, message).small().banner()
}

/// Inline informational banner.
pub fn info(id: impl Into<ElementId>, message: impl Into<Text>) -> Alert {
    Alert::info(id, message).small().banner()
}

/// Inline success banner.
pub fn success(id: impl Into<ElementId>, message: impl Into<Text>) -> Alert {
    Alert::success(id, message).small().banner()
}
