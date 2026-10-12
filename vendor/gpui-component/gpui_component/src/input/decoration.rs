//! Per-line visual decorations for read-only render surfaces such as the
//! diff viewer: a background fill plus a custom gutter string (e.g. dual
//! old/new line numbers), indexed by display row. Installed via
//! [`super::InputState::set_line_decorations`]. An empty decoration list
//! restores the editor's default behaviour — sequential `ix + 1` gutter,
//! no per-line background.

use gpui::{Hsla, SharedString};

/// Visual decoration applied to a single display row.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct LineDecoration {
    /// Full-width background fill behind the row (e.g. a diff add/remove
    /// tint). `None` leaves the row on the editor background.
    pub background: Option<Hsla>,
    /// Gutter text for this row, replacing the sequential `ix + 1`. The
    /// host pre-formats it — the diff viewer packs the old and new file
    /// line numbers into one column. When any row sets this, the editor
    /// sizes the gutter to the widest entry. `None` falls back to the
    /// row's sequential number.
    pub gutter: Option<SharedString>,
}
