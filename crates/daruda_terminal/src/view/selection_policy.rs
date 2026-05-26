//! iTerm2-style selection invalidation policy.
//!
//! Selection lives in absolute screen coordinates (`ScreenPos`), so it
//! naturally survives viewport scrolling and partial dirty-row repaints.
//! Selection must only be cleared when the underlying content the user
//! had highlighted is no longer reachable from those coordinates:
//!
//! - The whole viewport repainted in one frame (e.g. fast region scroll
//!   that we could not reconcile incrementally).
//! - The alt-screen was toggled (the screen buffer was swapped).
//! - A hard reset (`ESC c`, RIS) wiped terminal state.
//!
//! All other dirty patterns — including the high-frequency partial
//! repaints emitted by interactive shells like Claude Code's input box —
//! preserve the selection. This module exposes a single pure function
//! that returns the invalidation reason; the caller (the viewport
//! reconcile path) decides whether to clear.

use ghostty_vt::GridEvent;

/// Why the selection should be invalidated, or [`InvalidationReason::None`]
/// when it should be preserved.
#[derive(Debug, PartialEq, Eq)]
pub enum InvalidationReason {
    /// The dirty-row set covers the full viewport — we cannot tell which
    /// rows changed independently, so we conservatively clear.
    FullViewportDirty,
    /// The alt-screen was entered or exited; the underlying buffer
    /// changed identity.
    AltScreenToggle,
    /// A hard reset (RIS / `ESC c`) wiped terminal state.
    Ris,
    /// Selection should be preserved.
    None,
}

/// Decide whether a dirty viewport reconcile event should invalidate the
/// selection.
///
/// Priority: grid events (alt-screen / RIS) outrank dirty-row analysis,
/// because they indicate the buffer identity changed regardless of how
/// many rows were marked dirty. A zero-height viewport is treated as
/// [`InvalidationReason::None`] — there is nothing to invalidate against.
pub fn invalidation_reason(
    dirty_rows: &[u16],
    viewport_height: u16,
    grid_events: &[GridEvent],
) -> InvalidationReason {
    // Grid events outrank dirty analysis: alt-screen toggle / RIS imply
    // the buffer identity changed, so any selection anchor is stale even
    // if only a few rows happened to be marked dirty. RIS wins over an
    // alt-screen toggle in the same batch — ghostty emits a synthetic
    // `AltScreenToggle { entered: false }` just before `Ris` when the
    // hard reset leaves alt-screen, and we want the more specific signal.
    let mut alt_screen = false;
    let mut ris = false;
    for event in grid_events {
        match event {
            GridEvent::Ris => ris = true,
            GridEvent::AltScreenToggle { .. } => alt_screen = true,
        }
    }
    if ris {
        return InvalidationReason::Ris;
    }
    if alt_screen {
        return InvalidationReason::AltScreenToggle;
    }

    if viewport_height == 0 {
        return InvalidationReason::None;
    }

    // Full-viewport repaint: every row in the visible window is dirty.
    // ghostty_vt returns dirty rows uniquely per frame, so a `len ==
    // viewport_height` check is sufficient. Use `>=` defensively against
    // a future where duplicates could appear.
    if dirty_rows.len() as u32 >= viewport_height as u32 {
        return InvalidationReason::FullViewportDirty;
    }

    InvalidationReason::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_dirty_preserves_selection() {
        let dirty = vec![5, 6, 7];
        let reason = invalidation_reason(&dirty, 24, &[]);
        assert_eq!(reason, InvalidationReason::None);
    }

    #[test]
    fn full_viewport_dirty_invalidates() {
        let dirty: Vec<u16> = (0..24).collect();
        let reason = invalidation_reason(&dirty, 24, &[]);
        assert_eq!(reason, InvalidationReason::FullViewportDirty);
    }

    #[test]
    fn alt_screen_toggle_invalidates() {
        let reason = invalidation_reason(&[], 24, &[GridEvent::AltScreenToggle { entered: true }]);
        assert_eq!(reason, InvalidationReason::AltScreenToggle);
    }

    #[test]
    fn ris_invalidates() {
        let reason = invalidation_reason(&[], 24, &[GridEvent::Ris]);
        assert_eq!(reason, InvalidationReason::Ris);
    }

    #[test]
    fn zero_height_viewport_treated_as_none() {
        let reason = invalidation_reason(&[], 0, &[]);
        assert_eq!(reason, InvalidationReason::None);
    }
}
