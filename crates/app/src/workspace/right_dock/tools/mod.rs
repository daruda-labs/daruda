//! Tools tab — body renderer + CRUD modals.
//!
//! Module split mirrors `right_panel/skills/`:
//! - [`render`] — `right_panel::render` dispatch entry, draws the
//!   project + personal scope sections.
//! - [`add_modal`] / [`edit_modal`] / [`delete_confirm`] — modal
//!   openers as free functions. The openers are wrapped by
//!   `Workspace::open_*_mcp_*` methods in `ops`; `render.rs`
//!   dispatches through those wrappers.
//! - This `mod.rs` re-exports the openers so callers stay one-liners.

pub(super) mod add_modal;
pub(super) mod delete_confirm;
pub(super) mod edit_modal;
mod modal_shared;
mod ops;
pub(super) mod render;

pub(super) use add_modal::open_add_mcp_server_modal;
pub(super) use delete_confirm::open_delete_mcp_server_confirm;
pub(super) use edit_modal::open_edit_mcp_server_modal;
pub(in crate::workspace) use render::{footer, render};
