//! GPUI `Global` carrying the user config layer.
//!
//! Mirrors zed's settings-store shape: consumers subscribe via
//! `cx.observe_global::<SettingsStore>` and mutations go through
//! `cx.update_global` so observers fire on return. Each window resolves its
//! own project layer on top (`Workspace::apply_store_config`).

use daruda_config::{Config, SettingsPatch};
use gpui::{App, Global};
#[cfg(any(test, feature = "test-support"))]
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

/// Parsed user config and the writer that persists it.
pub struct SettingsStore {
    user: Arc<Config>,
    writer: ConfigWriter,
}

/// Ordered persistence boundary for user settings. GPUI's mutable global
/// update serializes access to this writer.
struct ConfigWriter {
    path: PathBuf,
    #[cfg(any(test, feature = "test-support"))]
    _temp_dir: Option<tempfile::TempDir>,
}

impl Default for ConfigWriter {
    fn default() -> Self {
        // In a test build, never the developer's own `config.toml`. A
        // fixture that read it would inherit whichever switches that person
        // happens to have on — the approval-card tests need Telegram *off*,
        // and a real user has it on — and one that patched it would rewrite
        // their settings. [`SettingsStore::set_user_for_testing`] replaces
        // this with a directory it seeds itself.
        #[cfg(any(test, feature = "test-support"))]
        {
            let temp_dir = tempfile::tempdir().expect("settings test temp dir");
            Self {
                path: temp_dir.path().join("config.toml"),
                _temp_dir: Some(temp_dir),
            }
        }
        #[cfg(not(any(test, feature = "test-support")))]
        Self {
            path: daruda_config::config_path(),
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

    fn reset_if_unchanged(
        &self,
        default: &SettingsPatch,
        expected: &Config,
    ) -> Result<Config, daruda_config::SettingsPatchApplyError> {
        daruda_config::reset_settings_field_to_if_unchanged(default, expected, &self.path)
    }
}

impl Global for SettingsStore {}

impl Default for SettingsStore {
    fn default() -> Self {
        Self {
            user: Arc::new(fresh_user_config()),
            writer: ConfigWriter::default(),
        }
    }
}

/// The user layer a fresh store starts from.
///
/// Defaults rather than the file in a test build, for the reason
/// [`ConfigWriter::default`] gives: a test must not depend on which switches
/// the developer running it has on.
fn fresh_user_config() -> Config {
    #[cfg(any(test, feature = "test-support"))]
    {
        Config::default()
    }
    #[cfg(not(any(test, feature = "test-support")))]
    {
        Config::load()
    }
}

impl SettingsStore {
    /// Idempotent initializer for production and test fixtures.
    pub fn init(cx: &mut App) {
        if !cx.has_global::<SettingsStore>() {
            cx.set_global(Self::default());
        }
    }

    /// Read-only access to the live Global; panics if [`Self::init`] never ran.
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

    /// [`Self::apply_patch_if_unchanged`] that deletes the field's key instead
    /// of writing `default`'s value. See
    /// [`daruda_config::reset_settings_field_to_if_unchanged`].
    pub fn reset_field_if_unchanged(
        &mut self,
        default: SettingsPatch,
        expected: &Config,
    ) -> Result<(), daruda_config::SettingsPatchApplyError> {
        match self.writer.reset_if_unchanged(&default, expected) {
            Ok(next) => {
                self.user = Arc::new(next);
                Ok(())
            }
            Err(error @ daruda_config::SettingsPatchApplyError::Conflict(_)) => {
                self.reload_user();
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    /// Replace the user layer for tests; production uses reload/patch paths.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn set_user_for_testing(&mut self, cfg: Config) {
        let temp_dir = tempfile::tempdir().expect("settings test tempdir");
        let path = temp_dir.path().join("config.toml");
        daruda_config::patch_config_file_to(&cfg, &path).expect("settings test config");
        // The catalog is read back: load promotes a custom entry that matches a
        // preset into a reference, and a live store only ever holds the loaded
        // form — kept as given, it would conflict with its own file. The rest
        // stays `cfg`, since this writer persists only the Settings-owned keys.
        let agents = Config::load_from(&path).agents;
        self.user = Arc::new(Config { agents, ..cfg });
        self.writer.path = path;
        self.writer._temp_dir = Some(temp_dir);
    }

    /// The temp `config.toml` a test store writes through. Lets a test
    /// corrupt that file to drive the persistence-failure branch of
    /// [`Self::apply_patch`] without a fake writer.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn writer_path_for_testing(&self) -> &Path {
        &self.writer.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_config::ShellConfig;
    use gpui::{BorrowAppContext, TestAppContext};

    /// `init(cx)` is idempotent — calling it twice does not overwrite
    /// an already-populated store. Mirrors the `tasks_global::init`
    /// A fixture must not inherit — or overwrite — the settings of whoever
    /// runs the suite. This is the guard for both halves: the read (a store
    /// starts at defaults, not at the file) and the write (the patch path
    /// addresses a temp file, not `config_path()`).
    #[gpui::test]
    fn a_test_store_never_addresses_the_developers_config(cx: &mut TestAppContext) {
        cx.update(|cx| {
            SettingsStore::init(cx);
            let store = SettingsStore::global(cx);
            assert_ne!(
                store.writer.path,
                daruda_config::config_path(),
                "a patch from a test must not land in the developer's config"
            );
            // Field-wise, because `Config` is not `PartialEq`: these are the
            // switches a real user turns on and a fixture must not see.
            let user = store.user();
            let default = Config::default();
            assert_eq!(user.telegram.enabled, default.telegram.enabled);
            assert_eq!(
                user.telegram.authorized_chat_id,
                default.telegram.authorized_chat_id
            );
            assert_eq!(
                user.orchestrator.enabled, default.orchestrator.enabled,
                "a fixture must not inherit whichever switches they have on"
            );
        });
    }

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
