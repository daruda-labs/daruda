//! GPUI Global wrapper for `daruda_store::tasks::TasksState`.
//!
//! `TasksState` is GPUI-free, so the orphan rule blocks a bare
//! `impl Global` from here. The newtype carries the marker plus
//! `Deref`/`DerefMut` so callers use it without `.0` boilerplate.

use std::ops::{Deref, DerefMut};
use std::path::Path;

use daruda_store::tasks::TasksState;
use gpui::{App, BorrowAppContext, Global};

/// Newtype over `TasksState` so the `impl Global` marker lives in the
/// app crate. Deref(Mut) makes call sites read as the inner state.
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

/// Register an empty `GlobalTasks` as the GPUI Global; Workspace
/// constructors then reload from their `data_dir` via [`load_from_dir`].
/// Idempotent so test fixtures that build a Workspace directly don't
/// panic on the first `cx.update_global` call.
pub fn init(cx: &mut App) {
    if !cx.has_global::<GlobalTasks>() {
        cx.set_global(GlobalTasks::default());
    }
}

/// Replace the global state with whatever is on disk under `data_dir`
/// (so per-test fixtures see their own task list). Falls back to the
/// default empty state on read failure.
pub fn load_from_dir(cx: &mut App, data_dir: &Path) {
    let state = daruda_store::tasks::load_tasks_in(data_dir).unwrap_or_default();
    if cx.has_global::<GlobalTasks>() {
        cx.update_global::<GlobalTasks, _>(|g, _| g.0 = state);
    } else {
        cx.set_global(GlobalTasks(state));
    }
}
