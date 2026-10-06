//! GPUI Global registration for [`SkillsState`]. The `impl Global` and
//! bootstrap helper live here so `agent/skills/mod.rs` stays GPUI-free
//! (CLAUDE.md G2 / G7).

use gpui::{App, Global};

use super::SkillsState;

impl Global for SkillsState {}

/// Register an empty `SkillsState` as the GPUI Global. Called from
/// `main.rs::app.run` before any Workspace; the first Workspace's
/// `refresh_skills_watcher` then populates it. Idempotent so test
/// fixtures building a Workspace directly don't panic on first
/// `cx.update_global`.
pub fn init(cx: &mut App) {
    if !cx.has_global::<SkillsState>() {
        cx.set_global(SkillsState::default());
    }
}
