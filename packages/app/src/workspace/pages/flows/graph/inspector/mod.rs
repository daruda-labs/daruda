//! GPUI-free inspector state. The view executes projection effects only.
pub(in crate::workspace) mod apply;
mod baseline;
mod draft;
mod effective;
pub(in crate::workspace) mod fields;
mod links;
mod model;
pub(in crate::workspace) mod notes;
mod notice;
mod prompt;
mod raw;
mod transitions;

pub(in crate::workspace) use draft::NodeDraft;
pub(in crate::workspace) use model::{
    Effects, FormChrome, InspectorModel, InspectorStatus, SavePlan,
};
pub(in crate::workspace) use notice::Notice;
pub(in crate::workspace) use prompt::{PromptEdit, PromptField};
pub(in crate::workspace) use raw::{RawInputs, UiHints};

#[cfg(test)]
mod tests;
