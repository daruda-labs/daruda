//! GPUI Global wrapper for `daruda_store::tasks::TasksState`.
//!
//! `TasksState` is defined in `daruda_store` (GPUI-free), so Rust's
//! orphan rule blocks a bare `impl Global for TasksState` from here.
//! The newtype below carries the marker impl plus `Deref`/`DerefMut`
//! so callers use the wrapped state through the usual
//! `cx.global::<GlobalTasks>()` pattern without `.0` boilerplate.
//!
//! Mirrors Zed's `GlobalAppState(Arc<AppState>)` wrapper pattern at
//! `crates/workspace/src/workspace.rs:1079`.

use std::ops::{Deref, DerefMut};
use std::path::Path;

use daruda_store::tasks::TasksState;
use gpui::{App, BorrowAppContext, Global};

/// Newtype over `daruda_store::tasks::TasksState` so the `impl Global`
/// marker lives in the app crate. Deref(Mut) makes call sites
/// indistinguishable from owning the inner state directly.
#[derive(Default)]
pub struct GlobalTasks(pub(crate) TasksState);

impl Global for GlobalTasks {}

impl Deref for GlobalTasks {
    type Target = TasksState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for GlobalTasks {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Register an empty `GlobalTasks` as the GPUI Global. Workspace
/// constructors then reload from their `data_dir` via [`load_from_dir`]
/// — production paths all use the default `~/.config/daruda/`.
///
/// Idempotent — mirrors `gpui_component::theme::Theme::change`'s
/// `has_global` guard so test fixtures that build a Workspace
/// directly (without going through `init_gpui_component`) don't
/// panic on the first `cx.update_global::<GlobalTasks, _>` call.
pub fn init(cx: &mut App) {
    if !cx.has_global::<GlobalTasks>() {
        cx.set_global(GlobalTasks::default());
    }
}

/// Replace the global state with whatever is on disk under
/// `data_dir`. Used by Workspace constructors so test fixtures
/// (which spin up a fresh `data_dir` per test) see their own task
/// list. Falls back to the default (empty) state on read failure.
pub fn load_from_dir(cx: &mut App, data_dir: &Path) {
    let state = daruda_store::tasks::load_tasks_in(data_dir).unwrap_or_default();
    if cx.has_global::<GlobalTasks>() {
        cx.update_global::<GlobalTasks, _>(|g, _| g.0 = state);
    } else {
        cx.set_global(GlobalTasks(state));
    }
}
