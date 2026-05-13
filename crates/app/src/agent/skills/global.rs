//! GPUI Global registration for [`SkillsState`].
//!
//! `agent/skills/mod.rs` is GPUI-free (root CLAUDE.md G2 / G7). The
//! `impl Global` and the bootstrap helper live here so the data
//! module stays pure.

use gpui::{App, Global};

use super::SkillsState;

impl Global for SkillsState {}

/// Register an empty `SkillsState` as the GPUI Global. Call from
/// `main.rs::app.run` before any Workspace is constructed; the first
/// Workspace's `refresh_skills_watcher` then populates it via
/// `cx.update_global`.
///
/// Idempotent — mirrors `gpui_component::theme::Theme::change`'s
/// `has_global` guard so test fixtures that build a Workspace
/// directly (without going through `init_gpui_component`) don't
/// panic on the first `cx.update_global::<SkillsState, _>` call.
pub fn init(cx: &mut App) {
    if !cx.has_global::<SkillsState>() {
        cx.set_global(SkillsState::default());
    }
}
