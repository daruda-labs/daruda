//! Left dock — view tab strip plus per-view renderers.
//!
//! Hosts three swappable views (Lanes / Git / Files) from
//! `daruda_store::project::LeftDockView`; owns the header tab strip that
//! switches between them, with bodies rendered by `render.rs`.

use gpui::{Div, div, prelude::*};

pub(in crate::workspace) mod file_tree_context;
pub(in crate::workspace) mod file_tree_ops;
pub(super) mod files;
pub(super) mod git_changes;
pub(in crate::workspace) mod git_ops;
pub(super) mod projects;
pub(super) mod view_tabs;

/// Shared scaffold for a left-dock view body: a full-size vertical flex
/// column that clips overflow, giving all three views one definition for
/// sizing and overflow. Per-view concerns stay at the call site (Lanes
/// adds card `gap`; Git / Files chain `key_context` + `track_focus`).
pub(in crate::workspace) fn left_panel_body() -> Div {
    div().flex().flex_col().size_full().overflow_hidden()
}
