// SPDX-License-Identifier: AGPL-3.0-only
//! Viewport scroll-lock state that combines the "is the user reading
//! scrollback?" flag with the absolute-line anchor that keeps the
//! reading position stable across IND / SU grid scrolls.
//!
//! [`ViewportLock::Live`] means the viewport follows live PTY output.
//! [`ViewportLock::Pinned`] means the user has scrolled away; the
//! `anchor` field records the absolute line index of the viewport top
//! so [`crate::view::TerminalView`] can re-seek it whenever the grid
//! shifts underneath.
//!
//! The lock is released by:
//! - OSC 133 A (`check_prompt_arrived`) — the shell signals a new prompt.
//! - Alt-screen entry or exit — the screen buffer switches.
//! - RIS (full terminal reset) — the session is torn down.
//! - Explicit "scroll to bottom" action (`snap_to_bottom`).
//! - Any PTY input from the user (key press, macro, IME commit).

/// An ever-increasing absolute line index (`overflow + screen_row`).
pub type AbsLineIndex = u64;

/// Viewport scroll-lock state.
///
/// Combining the boolean "user scrolled" flag with the anchor into one
/// type makes the invariant `is_locked() == anchor().is_some()` a
/// structural fact rather than a caller convention.
#[derive(Default)]
pub enum ViewportLock {
    /// The viewport follows live PTY output (default).
    #[default]
    Live,
    /// The user has manually scrolled away from the bottom.
    /// `anchor` is the absolute line index of the viewport top row.
    Pinned { anchor: AbsLineIndex },
}

impl ViewportLock {
    /// Enter scroll-lock mode at `abs_y`.  Overwrites any existing anchor.
    pub fn lock(&mut self, abs_y: AbsLineIndex) {
        *self = Self::Pinned { anchor: abs_y };
    }

    /// Leave scroll-lock mode; the viewport resumes following live output.
    pub fn unlock(&mut self) {
        *self = Self::Live;
    }

    /// Return the current anchor, or `None` when the viewport is live.
    pub fn anchor(&self) -> Option<AbsLineIndex> {
        match self {
            Self::Pinned { anchor } => Some(*anchor),
            Self::Live => None,
        }
    }

    /// `true` when the viewport is locked to an absolute line.
    pub fn is_locked(&self) -> bool {
        matches!(self, Self::Pinned { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_live() {
        let lock = ViewportLock::default();
        assert!(!lock.is_locked());
        assert_eq!(lock.anchor(), None);
    }

    #[test]
    fn lock_stores_anchor() {
        let mut lock = ViewportLock::default();
        lock.lock(12345);
        assert!(lock.is_locked());
        assert_eq!(lock.anchor(), Some(12345));
    }

    #[test]
    fn unlock_returns_to_live() {
        let mut lock = ViewportLock::default();
        lock.lock(100);
        lock.unlock();
        assert!(!lock.is_locked());
        assert_eq!(lock.anchor(), None);
    }

    #[test]
    fn repeated_lock_overwrites_anchor() {
        let mut lock = ViewportLock::default();
        lock.lock(100);
        lock.lock(200);
        assert_eq!(lock.anchor(), Some(200));
    }

    #[test]
    fn unlock_when_already_live_is_noop() {
        let mut lock = ViewportLock::default();
        lock.unlock();
        assert!(!lock.is_locked());
    }

    #[test]
    fn is_locked_and_anchor_stay_in_sync() {
        // The invariant: is_locked() == anchor().is_some().
        // A Pinned state always has an anchor; Live never does.
        let mut lock = ViewportLock::default();
        assert_eq!(lock.is_locked(), lock.anchor().is_some());
        lock.lock(42);
        assert_eq!(lock.is_locked(), lock.anchor().is_some());
        lock.unlock();
        assert_eq!(lock.is_locked(), lock.anchor().is_some());
    }
}
