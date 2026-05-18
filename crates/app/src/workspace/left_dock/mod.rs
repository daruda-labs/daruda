//! Left-dock left-dock — view tab strip plus per-view renderers.
//!
//! The left dock hosts three swappable views (Worktrees / Git / Files)
//! picked from `daruda_store::project::LeftDockView`. This module owns the
//! header-mounted tab strip that switches between them; each view's
//! body is rendered from `render.rs` by matching on `left_dock_view`.

pub(in crate::workspace) mod file_tree_context;
pub(in crate::workspace) mod file_tree_ops;
pub(in crate::workspace) mod git_ops;
pub(super) mod files;
pub(super) mod git_changes;
pub(super) mod view_tabs;
pub(super) mod worktrees;
