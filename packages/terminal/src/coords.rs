//! Typed terminal row coordinates.
//!
//! The terminal juggles three vertical row spaces that are all small
//! integers, so the compiler cannot catch a mix-up on its own (see
//! `view/CLAUDE.md` "Coordinate spaces"). This module names them so the
//! ones that flow through shared APIs are self-documenting at the call
//! site.
//!
//! [`ViewportRow`] is the 0-based row of the **currently painted**
//! viewport — the unified scrollback + grid frame. It is *not* a ghostty
//! live-grid row: the two coincide only while `scroll_offset == 0`. The
//! distinction is exactly what the "input box afterimage" overlay bug
//! turned on.
//!
//! Note this newtype documents intent; it does **not** by itself prevent
//! the bug, because the ghostty FFI boundary takes a bare `u16` (a
//! `ViewportRow` unwraps to one). The mechanical guard that a viewport
//! row is dispatched on `scroll_offset` before reaching that FFI is
//! `scripts/lint-viewport-row-scroll.sh`.

/// 0-based row within the currently painted viewport (unified
/// scrollback + grid frame). Range `0..rows`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewportRow(u16);

impl ViewportRow {
    /// Wrap a 0-based viewport row index.
    pub fn new(row: u16) -> Self {
        Self(row)
    }

    /// The underlying 0-based index. Use at the point a raw `u16` is
    /// genuinely required (FFI, arithmetic) — never to bypass the
    /// scroll-offset dispatch in `TerminalSession`.
    pub fn get(self) -> u16 {
        self.0
    }
}

impl From<u16> for ViewportRow {
    fn from(row: u16) -> Self {
        Self(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_u16() {
        assert_eq!(ViewportRow::new(7).get(), 7);
        assert_eq!(ViewportRow::from(0u16).get(), 0);
    }
}
