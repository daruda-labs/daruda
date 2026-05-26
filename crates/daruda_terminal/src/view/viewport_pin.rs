// SPDX-License-Identifier: AGPL-3.0-only
//! Viewport anchor that pins the visible top row to a specific absolute line.
//!
//! When the user scrolls up, the viewport is pinned to the absolute line index
//! of the top row. Grid scrolls that occur during command execution (IND / SU)
//! preserve the anchor so the user's reading position stays fixed. The pin is
//! released only on an explicit user action (PTY input) or when the shell
//! signals a new prompt (OSC 133 A / `PromptStart`).

/// An ever-increasing absolute line index (`overflow + screen_row`).
pub type AbsLineIndex = u64;

/// Viewport anchor state.
///
/// `None` means the viewport follows the live terminal bottom (default).
/// `Some(abs)` means the top of the viewport should be locked to `abs`.
#[derive(Default)]
pub struct ViewportPin {
    anchor: Option<AbsLineIndex>,
}

impl ViewportPin {
    /// Set the anchor to `top_abs`.  Overwrites any existing anchor.
    pub fn pin(&mut self, top_abs: AbsLineIndex) {
        self.anchor = Some(top_abs);
    }

    /// Release the anchor.  After this call `is_pinned()` returns `false`.
    pub fn release(&mut self) {
        self.anchor = None;
    }

    /// Return the current anchor, or `None` when the viewport is following
    /// the live terminal bottom.
    pub fn anchor(&self) -> Option<AbsLineIndex> {
        self.anchor
    }

    /// `true` when the viewport is locked to an absolute line.
    pub fn is_pinned(&self) -> bool {
        self.anchor.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_stores_anchor() {
        let mut pin = ViewportPin::default();
        pin.pin(12345);
        assert_eq!(pin.anchor(), Some(12345));
        assert!(pin.is_pinned());
    }

    #[test]
    fn release_clears_anchor() {
        let mut pin = ViewportPin::default();
        pin.pin(100);
        pin.release();
        assert_eq!(pin.anchor(), None);
        assert!(!pin.is_pinned());
    }

    #[test]
    fn repeated_pin_overwrites() {
        let mut pin = ViewportPin::default();
        pin.pin(100);
        pin.pin(200);
        assert_eq!(pin.anchor(), Some(200));
    }
}
