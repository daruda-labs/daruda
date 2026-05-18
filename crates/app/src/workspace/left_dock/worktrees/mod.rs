//! Worktrees view — list of worktrees plus modals for create/remove.
//!
//! Renders into the left dock when `left_dock_view == Worktrees`. The
//! modal state lives on `Workspace` because it needs to outlive any
//! single render and survive view-tab switches.

pub(in crate::workspace) mod claude_badges;
pub(in crate::workspace) mod create_modal;
pub(in crate::workspace) mod list;
pub(in crate::workspace) mod merge_modal;
pub(in crate::workspace) mod remove_modal;

pub(in crate::workspace) use list::render;
