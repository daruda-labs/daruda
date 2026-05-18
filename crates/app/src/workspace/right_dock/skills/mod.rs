//! Skills tab — body renderer + CRUD modals.
//!
//! Module split:
//! - [`render`] — `right_panel::render` dispatch entry, draws the
//!   project + personal scope sections.
//! - [`create_modal`] / [`edit_modal`] / [`delete_confirm`] /
//!   [`rename_modal`] — modal openers as free functions. Callers
//!   (render handlers, `workspace/actions.rs::on_new_skill`) invoke
//!   them as `super::open_*_skill_*` rather than going through
//!   `Workspace` methods.
//! - This `mod.rs` re-exports the openers so callers stay one-liners.

pub(in crate::workspace) mod create_modal;
pub(in crate::workspace) mod delete_confirm;
pub(in crate::workspace) mod edit_modal;
pub(in crate::workspace) mod invocation_modal;
mod modal_shared;
pub(in crate::workspace) mod picker_modal;
pub(in crate::workspace) mod rename_modal;
pub(in crate::workspace) mod render;

pub(in crate::workspace) use create_modal::open_create_skill_modal;
pub(in crate::workspace) use delete_confirm::open_delete_skill_confirm;
pub(in crate::workspace) use edit_modal::open_edit_skill_modal;
pub(in crate::workspace) use invocation_modal::{SkillInvocationLabel, SkillInvocationModal};
pub(in crate::workspace) use picker_modal::SkillPickerModal;
pub(in crate::workspace) use rename_modal::open_rename_skill_modal;
pub(in crate::workspace) use render::render;
