//! Coordinate types for the interval tree.
//!
//! A `LineCoord` is either a scrollback-resident position (overflow-safe via
//! [`LineBufferPosition`]) or a viewport-only line still waiting to be
//! committed to the line buffer. Ordering places every `Buffered` coordinate
//! strictly before every `Viewport` coordinate so a half-line range that
//! starts in scrollback and ends in the live viewport remains well-formed.
//!
//! The tree itself never inspects these variants — it only requires `Ord`.

use serde::{Deserialize, Serialize};

use crate::session::line_buffer::LineBufferPosition;

/// Y-axis coordinate inside the unified line-buffer / viewport frame.
///
/// Comparison rule:
/// - Within `Buffered`, the inner `abs_index` (a monotonic absolute index
///   that survives ring eviction) is the ordering key.
/// - Within `Viewport`, `abs_y` is the ordering key.
/// - Across kinds, every `Buffered` is strictly less than every `Viewport`.
///
/// # Serde wire format
///
/// Externally-tagged: `{"Buffered":{"abs_index":42}}` or
/// `{"Viewport":{"abs_y":5}}`. This is a stable on-disk format; do not add
/// `#[serde(rename)]` or change variant names without a record-version bump.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineCoord {
    /// Scrollback-resident line whose handle is stable across ring trims.
    Buffered(LineBufferPosition),
    /// Viewport-only line. Rebound to `Buffered` once the line scrolls into
    /// the line buffer (handled by a later task, not the tree itself).
    Viewport { abs_y: u64 },
}

impl LineCoord {
    /// Inline key for ordering. Returns `(kind_rank, inner_index)` where
    /// `kind_rank == 0` for `Buffered` and `1` for `Viewport`.
    fn sort_key(&self) -> (u8, u64) {
        match self {
            LineCoord::Buffered(p) => (0, p.abs_index),
            LineCoord::Viewport { abs_y } => (1, *abs_y),
        }
    }
}

impl Ord for LineCoord {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

impl PartialOrd for LineCoord {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Inclusive interval `[start, end]` on the line axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    /// Lower bound of the range (inclusive).
    pub start: LineCoord,
    /// Upper bound of the range (inclusive).
    pub end: LineCoord,
}

impl LineRange {
    /// Construct a range. Panics in debug builds when `start > end`.
    pub fn new(start: LineCoord, end: LineCoord) -> Self {
        debug_assert!(start <= end, "LineRange::new requires start <= end");
        Self { start, end }
    }

    /// Two ranges overlap when their inclusive spans share at least one
    /// coordinate, i.e. `self.start <= other.end && other.start <= self.end`.
    pub fn overlaps(&self, other: &LineRange) -> bool {
        self.start <= other.end && other.start <= self.end
    }

    /// True when `line` falls inside `[start, end]` (both bounds inclusive).
    pub fn contains_line(&self, line: LineCoord) -> bool {
        self.start <= line && line <= self.end
    }
}
