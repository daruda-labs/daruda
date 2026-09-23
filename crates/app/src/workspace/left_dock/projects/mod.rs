//! Lanes view — list of lanes plus modals for create/remove.
//!
//! Renders into the left dock when `left_dock_view == Lanes`. The
//! modal state lives on `Workspace` because it needs to outlive any
//! single render and survive view-tab switches.

pub(super) mod agent_badges;
pub(super) mod banner;
pub(super) mod context_menu;
pub(super) mod create_modal;
pub(super) mod drag;
pub(super) mod group_menu;
pub(super) mod list;
pub(super) mod merge_modal;
mod modal_shared;
pub(super) mod project_menu;
pub(in crate::workspace) mod remove_modal;
pub(super) mod rows;
pub(super) mod session_host_modal;
pub(super) mod tree;

pub(in crate::workspace) use list::render;
