//! Tools tab — body renderer + CRUD modals.
//!
//! Module split mirrors `right_panel/skills/`:
//! - [`render`] — `right_panel::render` dispatch entry, draws the
//!   project + personal scope sections.
//! - [`add_modal`] / [`edit_modal`] / [`delete_confirm`] — modal
//!   openers as free functions. `render.rs` calls them as
//!   `super::open_*_mcp_*` rather than going through `Workspace`
//!   methods.
//! - This `mod.rs` re-exports the openers so callers stay one-liners.

pub(in crate::workspace) mod add_modal;
pub(in crate::workspace) mod delete_confirm;
pub(in crate::workspace) mod edit_modal;
mod modal_shared;
pub(in crate::workspace) mod render;

pub(in crate::workspace) use add_modal::open_add_mcp_server_modal;
pub(in crate::workspace) use delete_confirm::open_delete_mcp_server_confirm;
pub(in crate::workspace) use edit_modal::open_edit_mcp_server_modal;
pub(in crate::workspace) use render::render;
