//! GPUI `Global` carrying user config plus per-lane project overlays.
//!
//! Mirrors zed's settings-store shape: consumers subscribe via
//! `cx.observe_global::<SettingsStore>` and mutations go through
//! `cx.update_global` so observers fire on return. `init` stays idempotent and
//! removed lanes must call [`SettingsStore::forget_lane`] (CLAUDE.md
//! GPUI shared-state cleanup rule).

use crate::config_watcher;
use daruda_config::{Config, ProjectConfig, project};
use gpui::{App, BorrowAppContext as _, Global};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Parsed user config plus per-lane overlays keyed by absolute lane path.
pub struct SettingsStore {
    user: Arc<Config>,
    project: BTreeMap<PathBuf, Arc<ProjectConfig>>,
}

impl Global for SettingsStore {}

impl Default for SettingsStore {
    fn default() -> Self {
        Self {
            user: Arc::new(Config::load()),
            project: BTreeMap::new(),
        }
    }
}

impl SettingsStore {
    /// Idempotent initializer for production and test fixtures.
    pub fn init(cx: &mut App) {
        if !cx.has_global::<SettingsStore>() {
            cx.set_global(Self::default());
        }
    }

    /// Read-only access to the live Global; panics if [`init`] never ran.
    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Live user-layer snapshot.
    pub fn user(&self) -> &Config {
        &self.user
    }

    /// Cheap shared user-config clone for handlers outliving this borrow.
    pub fn user_arc(&self) -> Arc<Config> {
        self.user.clone()
    }

    /// Replace the cached user layer with a fresh `Config::load()` from disk.
    pub fn reload_user(&mut self) {
        self.user = Arc::new(Config::load());
    }

    /// Resolve owned effective config for a lane by applying its overlay.
    pub fn effective_for(&self, lane: Option<&Path>) -> Config {
        let base = (*self.user).clone();
        match lane.and_then(|w| self.project.get(w)) {
            Some(overlay) => base.resolve(overlay),
            None => base,
        }
    }

    /// Load or refresh the project overlay for `lane`.
    pub fn load_project_layer(&mut self, lane: &Path) {
        let cfg = ProjectConfig::load_for(lane);
        self.project.insert(lane.to_path_buf(), Arc::new(cfg));
    }

    /// Drop an overlay when a lane is removed (CLAUDE.md cleanup rule).
    pub fn forget_lane(&mut self, lane: &Path) {
        self.project.remove(lane);
    }

    /// Edit, persist, and cache-update the user layer in one step.
    pub fn patch_user<F>(&mut self, f: F) -> Result<(), String>
    where
        F: FnOnce(&mut Config),
    {
        let mut next = (*self.user).clone();
        f(&mut next);
        daruda_config::patch_config_file(&next)?;
        self.user = Arc::new(next);
        Ok(())
    }

    /// Replace the user layer for tests; production uses reload/patch paths.
    #[doc(hidden)]
    pub fn set_user_for_testing(&mut self, cfg: Config) {
        self.user = Arc::new(cfg);
    }
}

/// Spawn the `config.toml` watcher and refresh the global on debounced reloads.
/// Must run once after [`SettingsStore::init`]; the moved handle keeps the OS
/// watcher and debounce thread alive for the task lifetime.
pub fn spawn_file_watch(cx: &mut App) {
    let handle = config_watcher::spawn_config_watcher();
    let tick = std::time::Duration::from_millis(250);
    cx.spawn(async move |cx| {
        let handle = handle;
        loop {
            cx.background_executor().timer(tick).await;
            if handle.try_recv().is_err() {
                continue;
            }
            while handle.try_recv().is_ok() {}
            cx.update(|cx| {
                cx.update_global::<SettingsStore, _>(|store, _| store.reload_user());
            });
        }
    })
    .detach();
}

/// Convenience: probe the user config path. Mirrors [`daruda_config::config_path`]
/// but lives here too so consumers that already imported
/// `daruda_config::store::*` don't need a second `use`.
pub fn user_config_path() -> PathBuf {
    daruda_config::config_path()
}

/// Convenience: probe the per-lane config path. Mirrors
/// [`project::project_config_path`]; returns `None` when the
/// lane resides outside the user's home or otherwise has no
/// resolvable XDG-aligned project dir.
pub fn lane_config_path(lane: &Path) -> Option<PathBuf> {
    project::project_config_path(lane)
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_config::ShellConfig;
    use gpui::{BorrowAppContext, TestAppContext};

    /// `init(cx)` is idempotent — calling it twice does not overwrite
    /// an already-populated store. Mirrors the `tasks_global::init`
    /// fixture expectation that production entry + test setup both
    /// call `init` without panicking on the second `set_global`.
    #[gpui::test]
    fn init_is_idempotent(cx: &mut TestAppContext) {
        cx.update(|cx| {
            SettingsStore::init(cx);
            // Stamp a sentinel so we can detect a clobbering second init.
            cx.update_global::<SettingsStore, _>(|store, _| {
                let mut cfg = (*store.user).clone();
                cfg.shell = ShellConfig {
                    close_pane_on_exit: !cfg.shell.close_pane_on_exit,
                    ..cfg.shell.clone()
                };
                store.set_user_for_testing(cfg);
            });
            let sentinel = SettingsStore::global(cx).user().shell.close_pane_on_exit;

            SettingsStore::init(cx);
            assert_eq!(
                SettingsStore::global(cx).user().shell.close_pane_on_exit,
                sentinel,
                "second init() must not clobber the existing global"
            );
        });
    }

    /// `effective_for(None)` returns the user layer unchanged.
    #[gpui::test]
    fn effective_for_none_returns_user(cx: &mut TestAppContext) {
        cx.update(|cx| {
            SettingsStore::init(cx);
            let user = SettingsStore::global(cx).user().clone();
            let effective = SettingsStore::global(cx).effective_for(None);
            assert_eq!(
                effective.shell.close_pane_on_exit,
                user.shell.close_pane_on_exit
            );
        });
    }

    /// `forget_lane` drops the per-lane slice.
    #[gpui::test]
    fn forget_worktree_removes_entry(cx: &mut TestAppContext) {
        cx.update(|cx| {
            SettingsStore::init(cx);
            let path = PathBuf::from("/tmp/daruda-test-lane");
            cx.update_global::<SettingsStore, _>(|store, _| {
                store
                    .project
                    .insert(path.clone(), Arc::new(ProjectConfig::default()));
                assert!(store.project.contains_key(&path));
                store.forget_lane(&path);
                assert!(!store.project.contains_key(&path));
            });
        });
    }

    /// `observe_global` fires after `cx.update_global` returns and
    /// the effect queue drains. Drains by splitting the test into
    /// two `cx.update` closures — GPUI flushes effects on the
    /// outer-update boundary. Guards the A2 step: workspace will
    /// subscribe via this same mechanism.
    #[gpui::test]
    fn observe_global_fires_on_update(cx: &mut TestAppContext) {
        use std::cell::Cell;
        use std::rc::Rc;

        let count = Rc::new(Cell::new(0u32));

        let subscription = cx.update(|cx| {
            SettingsStore::init(cx);
            let count_for_cb = count.clone();
            cx.observe_global::<SettingsStore>(move |_cx| {
                count_for_cb.set(count_for_cb.get() + 1);
            })
        });

        cx.update(|cx| {
            cx.update_global::<SettingsStore, _>(|store, _| {
                let mut cfg = (*store.user).clone();
                cfg.shell.close_pane_on_exit = !cfg.shell.close_pane_on_exit;
                store.set_user_for_testing(cfg);
            });
        });

        cx.run_until_parked();
        assert_eq!(count.get(), 1, "observer must fire after update_global");
        drop(subscription);
    }
}
