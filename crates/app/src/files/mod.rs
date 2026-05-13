//! GPUI-free file-tree primitives for the sidebar Files view.
//!
//! Layout follows the rest of the crate's GPUI-free modules
//! (`worktree::git`, `agent::skills`): pure data structures and
//! blocking helpers live here, the `Workspace` glue lives in
//! `workspace::file_tree_ops`, and the renderer lives in
//! `workspace::sidebar::files`.

pub mod gitignore;
pub mod icons;
pub mod load;
pub mod tree;
pub mod watcher;
