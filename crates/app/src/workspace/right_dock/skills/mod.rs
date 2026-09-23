//! Skills tab — body renderer + CRUD modals.
//!
//! Module split:
//! - [`render`] — `right_panel::render` dispatch entry, draws the
//!   project + personal scope sections.
//! - [`create_modal`] / [`edit_modal`] / [`delete_confirm`] /
//!   [`rename_modal`] — modal openers as free functions. The openers
//!   are wrapped by `Workspace::open_*_skill[_confirm]` methods in
//!   `ops`; `render.rs` dispatches through those wrappers (other call
//!   sites such as `workspace/actions.rs::on_new_skill` still use the
//!   free-fn re-exports).
//! - This `mod.rs` re-exports the openers so callers stay one-liners.

pub(super) mod create_modal;
pub(super) mod delete_confirm;
pub(super) mod edit_modal;
pub(super) mod invocation_modal;
mod modal_shared;
mod ops;
pub(super) mod picker_modal;
pub(super) mod rename_modal;
pub(super) mod render;

pub(in crate::workspace) use create_modal::open_create_skill_modal;
pub(super) use delete_confirm::open_delete_skill_confirm;
pub(super) use edit_modal::open_edit_skill_modal;
pub(super) use invocation_modal::{SkillInvocationLabel, SkillInvocationModal};
pub(super) use picker_modal::SkillPickerModal;
pub(super) use rename_modal::open_rename_skill_modal;
pub(in crate::workspace) use render::{footer, render};
