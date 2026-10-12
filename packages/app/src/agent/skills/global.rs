//! GPUI Global wrapper for [`SkillsState`].
//!
//! `SkillsState` is GPUI-free, so the orphan rule blocks a bare
//! `impl Global` from here. The newtype carries the marker plus
//! `Deref`/`DerefMut` so callers use it without `.0` boilerplate.

use std::ops::{Deref, DerefMut};

use gpui::{App, Global};

use super::SkillsState;

/// Newtype over `SkillsState` so the `impl Global` marker lives in the app
/// crate.
#[derive(Default)]
pub struct GlobalSkills(pub(crate) SkillsState);

impl Global for GlobalSkills {}

impl Deref for GlobalSkills {
    type Target = SkillsState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for GlobalSkills {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Register an empty `GlobalSkills`. Called from `main.rs::app.run` before
/// any Workspace; the first Workspace's `refresh_skills_watcher` then
/// populates it. Idempotent so test fixtures building a Workspace directly
/// don't panic on first `cx.update_global`.
pub fn init(cx: &mut App) {
    if !cx.has_global::<GlobalSkills>() {
        cx.set_global(GlobalSkills::default());
    }
}
