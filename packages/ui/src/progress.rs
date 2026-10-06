//! Progress-bar wrapper over `gpui_component::progress::Progress`.
//!
//! `Progress` is `Styled` (not `Sizable`), so the factory just seeds
//! the value; callers chain `.bg(color)` and the usual Styled metrics
//! (`.h(...)`, `.rounded(...)`). The fill defaults to
//! `theme.progress_bar` when no `.bg(...)` is set.

pub use gpui_component::progress::Progress;

/// A progress bar filled to `value` (clamped to `0.0..=100.0` by the
/// widget). Chain `.bg(color)` to override the fill and Styled methods
/// for height / corner radius.
pub fn progress(value: f32) -> Progress {
    Progress::new().value(value)
}
