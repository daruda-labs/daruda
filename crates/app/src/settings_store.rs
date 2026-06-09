//! `SettingsStore` — single GPUI `Global` carrying the user [`Config`]
//! plus per-lane [`ProjectConfig`] overlays.
//!
//! Mirrors the zed `crates/settings/src/settings_store.rs` pattern:
//! a Global owning both layers, with consumers subscribing via
//! `cx.observe_global::<SettingsStore>` instead of having a watcher
//! call into each [`Workspace`] one-by-one.
//!
//! Bootstrap rules (CLAUDE.md §"GPUI shared-state convention"):
//!
//! - [`init`] is idempotent (`cx.has_global` guard) so test fixtures
//!   and the production entry point both call it without panicking
//!   on the second `set_global`.
//! - Mutate via `cx.update_global::<SettingsStore, _>(|store, _| ...)`
//!   so observers fire automatically on the closure return.
//! - Per-lane slices live in a `BTreeMap<PathBuf, ...>` and
//!   every `Workspace::finalize_remove_lane` must call
//!   [`SettingsStore::forget_lane`] so the map can't grow
//!   unbounded across a long session.

use crate::config_watcher;
use daruda_config::{Config, ProjectConfig, project};
use gpui::{App, BorrowAppContext as _, Global};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Settings layers — user-global plus per-lane overlay.
///
/// `user` is the parsed `~/.config/daruda/config.toml`. `project`
/// keys are absolute lane paths and the values are the parsed
/// `.daruda/config.toml` overlay sitting inside each lane.
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
    /// Idempotent initialiser. Production entry point
    /// (`globals::init_all`) and test fixtures both call this; the
    /// `has_global` guard keeps a second call from clobbering an
    /// already-populated store.
    pub fn init(cx: &mut App) {
        if !cx.has_global::<SettingsStore>() {
            cx.set_global(Self::default());
        }
    }

    /// Read-only access to the live Global. Panics if [`init`] never
    /// ran — call from the GPUI app context only.
    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Live user-layer snapshot. Returns a reference into the Arc
    /// the store holds; callers cloning the `Arc` get cheap shared
    /// ownership without re-reading disk.
    pub fn user(&self) -> &Config {
        &self.user
    }

    /// User config wrapped in `Arc`. Cheap clone for handlers that
    /// need to outlive the borrow on `SettingsStore`.
    pub fn user_arc(&self) -> Arc<Config> {
        self.user.clone()
    }

    /// Replace the cached user layer with a fresh `Config::load()`
    /// from disk. Call via `cx.update_global::<SettingsStore, _>` so
    /// `observe_global` callbacks fire on return.
    pub fn reload_user(&mut self) {
        self.user = Arc::new(Config::load());
    }

    /// Resolve the effective `Config` for an optional lane path
    /// by composing the user layer with the matching project
    /// overlay (if any).  Falls back to the user layer when
    /// `lane` is `None` or no overlay has been loaded for that
    /// path. Returns a fresh owned `Config` rather than a borrow so
    /// callers can hand it to `Workspace::apply_config`.
    pub fn effective_for(&self, lane: Option<&Path>) -> Config {
        let base = (*self.user).clone();
        match lane.and_then(|w| self.project.get(w)) {
            Some(overlay) => base.resolve(overlay),
            None => base,
        }
    }

    /// Load (or refresh) the project overlay for `lane`. Reads
    /// from disk using the daruda-config conventions
    /// (`project_config_path`).  Idempotent — safe to call from
    /// `Workspace::new_with_project` and again from any reload path.
    pub fn load_project_layer(&mut self, lane: &Path) {
        let cfg = ProjectConfig::load_for(lane);
        self.project.insert(lane.to_path_buf(), Arc::new(cfg));
    }

    /// Drop the project overlay for `lane`. Required by
    /// `Workspace::finalize_remove_lane` so the map can't grow
    /// unbounded across a long session (CLAUDE.md "Cleanup rule").
    pub fn forget_lane(&mut self, lane: &Path) {
        self.project.remove(lane);
    }

    /// Surgical user-config edit + persist + cache update. Pass the
    /// closure through `cx.update_global::<SettingsStore, _>` so the
    /// `observe_global` fanout runs on return. Returns the same
    /// `Err` shape as [`daruda_config::patch_config_file`] for the caller to
    /// surface as an `ErrorReport` if writing fails.
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

    /// Replace the user layer with an explicit value — testing
    /// helper.  Production should go through [`reload_user`] or
    /// [`patch_user`]. Mirrors zed's `update_user_settings` helper.
    #[doc(hidden)]
    pub fn set_user_for_testing(&mut self, cfg: Config) {
        self.user = Arc::new(cfg);
    }
}

/// Spawn the filesystem watcher for `config.toml` and a background
/// pump that, for every debounced reload signal, refreshes the
/// `SettingsStore` Global. Workspace `cx.observe_global` callbacks
/// fan out the change to every open window.
///
/// Must be called exactly once during app bootstrap, *after*
/// [`SettingsStore::init`].
///
/// The watcher handle is moved into the spawned task; dropping the
/// task drops the handle, which releases the live `notify::Watcher`
/// and ends the debounce thread (RAII end-to-end). The sync
/// receiver is polled via `try_recv` on a 250 ms timer — the
/// channel pre-debounces inside `spawn_config_watcher`, and any
/// leftover burst is drained before a single `reload_user` apply.
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
