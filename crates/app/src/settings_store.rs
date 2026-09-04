//! GPUI `Global` carrying user config plus per-lane project overlays.
//!
//! Mirrors zed's settings-store shape: consumers subscribe via
//! `cx.observe_global::<SettingsStore>` and mutations go through
//! `cx.update_global` so observers fire on return. `init` stays idempotent and
//! removed lanes must call [`SettingsStore::forget_lane`] (CLAUDE.md
//! GPUI shared-state cleanup rule).

use crate::config_watcher;
use daruda_config::{Config, ProjectConfig, SettingsPatch, project};
use gpui::{App, BorrowAppContext as _, Global};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Parsed user config plus per-lane overlays keyed by absolute lane path.
pub struct SettingsStore {
    user: Arc<Config>,
    project: BTreeMap<PathBuf, Arc<ProjectConfig>>,
    writer: ConfigWriter,
}

/// Ordered persistence boundary for user settings. GPUI's mutable global
/// update serializes access to this writer.
struct ConfigWriter {
    path: PathBuf,
    #[cfg(test)]
    _temp_dir: Option<tempfile::TempDir>,
}

impl Default for ConfigWriter {
    fn default() -> Self {
        Self {
            path: daruda_config::config_path(),
            #[cfg(test)]
            _temp_dir: None,
        }
    }
}

impl ConfigWriter {
    fn write(&self, patch: &SettingsPatch) -> Result<Config, String> {
        daruda_config::apply_settings_patch_to(patch, &self.path)
    }

    fn write_if_unchanged(
        &self,
        patch: &SettingsPatch,
        expected: &Config,
    ) -> Result<Config, daruda_config::SettingsPatchApplyError> {
        daruda_config::apply_settings_patch_to_if_unchanged(patch, expected, &self.path)
    }
}

impl Global for SettingsStore {}

impl Default for SettingsStore {
    fn default() -> Self {
        Self {
            user: Arc::new(Config::load()),
            project: BTreeMap::new(),
            writer: ConfigWriter::default(),
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

    /// Replace the cached user layer with a fresh config from disk.
    pub fn reload_user(&mut self) {
        self.user = Arc::new(Config::load_from(&self.writer.path));
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

    /// Persist one field or structural catalog against the latest TOML, then
    /// publish the exact config that was written.
    pub fn apply_patch(&mut self, patch: SettingsPatch) -> Result<(), String> {
        let next = self.writer.write(&patch)?;
        self.user = Arc::new(next);
        Ok(())
    }

    /// Settings-window form of [`Self::apply_patch`]. The writer compares the
    /// addressed field with `expected` in the document it actually read, so a
    /// watcher debounce cannot hide a same-field external edit.
    pub fn apply_patch_if_unchanged(
        &mut self,
        patch: SettingsPatch,
        expected: &Config,
    ) -> Result<(), daruda_config::SettingsPatchApplyError> {
        match self.writer.write_if_unchanged(&patch, expected) {
            Ok(next) => {
                self.user = Arc::new(next);
                Ok(())
            }
            Err(error @ daruda_config::SettingsPatchApplyError::Conflict(_)) => {
                // The writer observed a newer valid document than this cache.
                // Publish it immediately so the conflict banner's "Use
                // external value" action does not wait for watcher debounce.
                self.reload_user();
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    /// Replace the user layer for tests; production uses reload/patch paths.
    #[cfg(test)]
    #[doc(hidden)]
    pub fn set_user_for_testing(&mut self, cfg: Config) {
        let temp_dir = tempfile::tempdir().expect("settings test tempdir");
        let path = temp_dir.path().join("config.toml");
        daruda_config::patch_config_file_to(&cfg, &path).expect("settings test config");
        self.writer.path = path;
        self.writer._temp_dir = Some(temp_dir);
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

    #[gpui::test]
    fn apply_patch_reloads_latest_file_before_writing(cx: &mut TestAppContext) {
        cx.update(|cx| {
            SettingsStore::init(cx);
            cx.update_global::<SettingsStore, _>(|store, _| {
                store.set_user_for_testing(Config::default());
                daruda_config::apply_settings_patch_to(
                    &SettingsPatch::EditorFontSize(17.0),
                    &store.writer.path,
                )
                .expect("external edit");

                store
                    .apply_patch(SettingsPatch::TerminalFontSize(16.0))
                    .expect("settings edit");

                assert_eq!(store.user().font.terminal.size, 16.0);
                assert_eq!(store.user().font.editor.size, 17.0);
                let disk = Config::load_from(&store.writer.path);
                assert_eq!(disk.font.terminal.size, 16.0);
                assert_eq!(disk.font.editor.size, 17.0);
            });
        });
    }

    #[gpui::test]
    fn checked_patch_detects_a_disk_change_before_the_watcher_reload(cx: &mut TestAppContext) {
        cx.update(|cx| {
            SettingsStore::init(cx);
            cx.update_global::<SettingsStore, _>(|store, _| {
                store.set_user_for_testing(Config::default());
                let baseline = store.user().clone();
                std::fs::write(&store.writer.path, "[font]\nsize = 20.0\n").expect("external edit");

                let error = store
                    .apply_patch_if_unchanged(SettingsPatch::TerminalFontSize(16.0), &baseline)
                    .expect_err("stale cache must not hide the disk conflict");

                assert_eq!(
                    error,
                    daruda_config::SettingsPatchApplyError::Conflict(
                        daruda_config::SettingsFieldId::TerminalFontSize
                    )
                );
                assert_eq!(store.user().font.terminal.size, 20.0);
                assert_eq!(
                    Config::load_from(&store.writer.path).font.terminal.size,
                    20.0
                );
            });
        });
    }
}
