//! The strings that need logic — plural choice, an argument formatted before
//! it is interpolated. Each file is re-exported into its section's generated
//! module, and a function named like a key replaces the generated one.

pub(super) mod agent_chat;
pub(super) mod bottom_dock;
pub(super) mod branch_rule;
pub(super) mod command;
pub(super) mod control;
pub(super) mod ctx;
pub(super) mod file_viewer;
pub(super) mod flow;
pub(super) mod git;
pub(super) mod mcp;
pub(super) mod menu;
pub(super) mod modal;
pub(super) mod notification;
pub(super) mod session_host;
pub(super) mod settings;
pub(super) mod status;
pub(super) mod task;
pub(super) mod usage;
