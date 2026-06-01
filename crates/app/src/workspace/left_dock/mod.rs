//! Left-dock left-dock — view tab strip plus per-view renderers.
//!
//! The left dock hosts three swappable views (Lanes / Git / Files)
//! picked from `daruda_store::project::LeftDockView`. This module owns the
//! header-mounted tab strip that switches between them; each view's
//! body is rendered from `render.rs` by matching on `left_dock_view`.

use gpui::{Div, div, prelude::*};

pub(in crate::workspace) mod file_tree_context;
pub(in crate::workspace) mod file_tree_ops;
pub(super) mod files;
pub(super) mod git_changes;
pub(in crate::workspace) mod git_ops;
pub(super) mod projects;
pub(super) mod view_tabs;

/// Shared scaffold for a left-dock view body: a full-size vertical flex
/// column that clips overflow. The three left-dock views (Lanes / Git /
/// Files) build their body on this so sizing and overflow handling have
/// a single definition rather than three near-identical copies.
/// Per-view concerns stay at the call site: the Lanes tree adds its card
/// `gap`, and the Git / Files panels chain `key_context` + `track_focus`
/// for keyboard navigation.
pub(in crate::workspace) fn left_panel_body() -> Div {
    div().flex().flex_col().size_full().overflow_hidden()
}
