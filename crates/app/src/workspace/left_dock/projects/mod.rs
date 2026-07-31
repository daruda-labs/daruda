//! Lanes view — list of lanes plus modals for create/remove.
//!
//! Renders into the left dock when `left_dock_view == Lanes`. The
//! modal state lives on `Workspace` because it needs to outlive any
//! single render and survive view-tab switches.

pub(in crate::workspace) mod agent_badges;
pub(in crate::workspace) mod banner;
pub(in crate::workspace) mod card;
pub(in crate::workspace) mod context_menu;
pub(in crate::workspace) mod create_modal;
pub(in crate::workspace) mod drag;
pub(in crate::workspace) mod group_menu;
pub(in crate::workspace) mod list;
pub(in crate::workspace) mod merge_modal;
pub(in crate::workspace) mod project_menu;
pub(in crate::workspace) mod remove_modal;
pub(in crate::workspace) mod rows;
pub(in crate::workspace) mod session_host_modal;

pub(in crate::workspace) use list::render;
