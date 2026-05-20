//! `Workspace` method wrappers over the skills modal free functions.
//!
//! Keeps `render.rs` closures one-liners dispatching through `Workspace`
//! rather than calling `super::open_*` free functions directly.

use std::path::PathBuf;

use gpui::{Context, Window};

use crate::agent::skills::SkillScope;
use crate::workspace::Workspace;

impl Workspace {
    pub(in crate::workspace) fn open_create_skill(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::open_create_skill_modal(self, None, window, cx);
    }

    pub(in crate::workspace) fn open_edit_skill(
        &mut self,
        dir: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::open_edit_skill_modal(self, dir, window, cx);
    }

    pub(in crate::workspace) fn open_delete_skill_confirm(
        &mut self,
        scope: SkillScope,
        dir: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::open_delete_skill_confirm(self, scope, dir, window, cx);
    }
}

#[cfg(test)]
mod tests {
    // Behavioral coverage for these wrappers requires a live `Window`
    // and `Context<Workspace>`; the underlying modal-open logic is
    // exercised by integration tests on the modal modules themselves.
}
