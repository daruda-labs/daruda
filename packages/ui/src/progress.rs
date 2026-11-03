//! Progress-bar wrapper over `gpui_component::progress::Progress`.
//!
//! The factory seeds the id and value; callers chain `.color(color)` for the
//! fill (the track is that colour at 20% opacity) and the usual Styled metrics
//! (`.h(...)`, `.rounded(...)`). The fill defaults to `theme.progress_bar`.
//! The id keys the bar's value transition, so it must be unique on screen.

pub use gpui_component::progress::Progress;

/// A progress bar filled to `value` (clamped to `0.0..=100.0` by the
/// widget). Chain `.color(color)` to override the fill and Styled methods
/// for height / corner radius.
pub fn progress(id: impl Into<gpui::ElementId>, value: f32) -> Progress {
    Progress::new(id).value(value)
}
