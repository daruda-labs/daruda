//! File-system watcher for `config.toml` live reload.
//!
//! Watches the user-config directory recursively so that any
//! `config.toml` under it (the user-global file plus per-project
//! files at `projects/<repo>-<hash>/config.toml`) triggers a reload.
//! The recursive scope keeps a single watcher thread responsible for
//! the entire layered-config tree; a filename filter ignores other
//! state files (`recent.json`, `state.json`, etc.) under the same
//! root. The reload signal is debounced for 200 ms.
//!
//! The pump in [`crate::settings_store::spawn_file_watch`] consumes
//! the debounced channel and folds each tick into a
//! `cx.update_global::<SettingsStore, _>` mutation — the user
//! layer is refreshed in place and every workspace that subscribed
//! via `cx.observe_global` re-evaluates `effective_for` on the next
//! tick.
//!
//! ## Lifecycle
//!
//! [`spawn_config_watcher`] returns a [`ConfigWatcherHandle`] that
//! couples the `Receiver<()>` with the live `notify::Watcher`.
//! Callers keep the handle alive as long as they poll; dropping it
//! shuts the kernel-side watch + the debounce thread cleanly.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;

/// Minimum interval between successive reloads. File-save events
/// often arrive in bursts (write + rename + chmod); debouncing
/// prevents flicker from partial writes.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(200);

/// RAII handle returned by [`spawn_config_watcher`]. Owns the live
/// `notify::Watcher` so the kernel-side subscription stays attached
/// for the handle's lifetime — dropping the handle ends the watch
/// and lets the debounce thread exit cleanly via channel close.
///
/// The handle dereferences to the debounced `Receiver<()>` so most
/// call sites read like the previous bare-receiver API:
/// `if handle.try_recv().is_err() { ... }`.
pub struct ConfigWatcherHandle {
    debounced_rx: mpsc::Receiver<()>,
    /// `Option` so [`Drop`] can take ownership and drop the watcher
    /// before the receiver — guarantees the FS callback's send-side
    /// closes first, which then ends the debounce thread's
    /// `rx.recv()` loop on the next event.
    ///
    /// `Box<dyn>` because `notify::recommended_watcher` returns a
    /// platform-specific concrete type that we don't want to leak
    /// into the public signature.
    _watcher: Option<Box<dyn notify::Watcher + Send>>,
}

impl ConfigWatcherHandle {
    /// Non-blocking poll for a debounced reload signal.
    pub fn try_recv(&self) -> Result<(), mpsc::TryRecvError> {
        self.debounced_rx.try_recv()
    }
}

impl Drop for ConfigWatcherHandle {
    fn drop(&mut self) {
        // Explicit drop order: watcher first, then the receiver via
        // `Self`'s normal drop. Dropping the watcher closes its
        // callback's `Sender`, which lets the debounce thread's
        // `rx.recv()` return `Err` and exit cleanly.
        let _ = self._watcher.take();
    }
}

/// Start watching the config file. Returns a [`ConfigWatcherHandle`]
/// the caller must keep alive for as long as it wants reload
/// notifications. Dropping the handle ends the kernel-side watch +
/// the debounce thread.
pub fn spawn_config_watcher() -> ConfigWatcherHandle {
    use notify::{RecursiveMode, Watcher};

    let config_path = daruda_config::config_path();
    let watch_dir = config_path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let file_name = config_path.file_name().map(|n| n.to_os_string());

    // `tx` is moved into the FS callback; dropping the watcher drops
    // the callback, which drops `tx`, which closes `rx`. That signal
    // is what shuts the debounce thread down cleanly at handle-drop
    // time.
    let (tx, rx) = mpsc::channel();
    let mut watcher: Box<dyn Watcher + Send> =
        match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                // Only trigger on the config file itself.
                let is_relevant = file_name.as_ref().is_none_or(|target| {
                    event
                        .paths
                        .iter()
                        .any(|p| p.file_name() == Some(target.as_os_str()))
                });
                if is_relevant {
                    let _ = tx.send(());
                }
            }
        }) {
            Ok(w) => Box::new(w),
            Err(e) => {
                LogWriter::log(
                    ErrorReport::new("Config watcher disabled — FS watcher init failed")
                        .severity(ErrorSeverity::Warning)
                        .from_error(&e)
                        .at(file!(), line!())
                        .dedup("config.watcher.init")
                        .build(),
                );
                // Inert handle: an empty receiver makes the pump's
                // `try_recv` always return `Empty`, so config-watch
                // is silently disabled.
                let (_dead_tx, dead_rx) = mpsc::channel();
                return ConfigWatcherHandle {
                    debounced_rx: dead_rx,
                    _watcher: None,
                };
            }
        };

    // Recursive so the same watcher picks up
    // `<config_dir>/daruda/projects/<...>/config.toml` writes
    // alongside the user-global file. The filename filter above
    // gates which events actually fire reloads.
    if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::Recursive) {
        LogWriter::log(
            ErrorReport::new("Config watcher disabled — could not attach to directory")
                .severity(ErrorSeverity::Warning)
                .from_error(&e)
                .at(file!(), line!())
                .with_context("path", redact_home(&watch_dir))
                .dedup("config.watcher.attach")
                .build(),
        );
        let (_dead_tx, dead_rx) = mpsc::channel();
        return ConfigWatcherHandle {
            debounced_rx: dead_rx,
            _watcher: None,
        };
    }

    // Debounce thread: collapse a burst into one signal. Exits when
    // `rx.recv()` returns `Err`, which happens after the watcher is
    // dropped and its callback's `Sender` goes out of scope.
    let (debounced_tx, debounced_rx) = mpsc::channel();
    std::thread::spawn(move || {
        loop {
            if rx.recv().is_err() {
                break;
            }
            std::thread::sleep(DEBOUNCE);
            while rx.try_recv().is_ok() {}
            if debounced_tx.send(()).is_err() {
                break;
            }
        }
    });

    ConfigWatcherHandle {
        debounced_rx,
        _watcher: Some(watcher),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn debounce_constant_is_reasonable() {
        assert!(DEBOUNCE.as_millis() >= 50);
        assert!(DEBOUNCE.as_millis() <= 1000);
    }

    #[test]
    fn config_reload_produces_valid_config() {
        let dir = std::env::temp_dir().join("daruda_config_reload_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "[font]").unwrap();
        writeln!(f, "size = 18.0").unwrap();
        drop(f);

        let cfg = daruda_config::Config::load_from(&path);
        assert_eq!(cfg.font.size, 18.0);

        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "[font]").unwrap();
        writeln!(f, "size = 22.0").unwrap();
        drop(f);

        let cfg2 = daruda_config::Config::load_from(&path);
        assert_eq!(cfg2.font.size, 22.0);

        let _ = std::fs::remove_dir_all(dir);
    }
}
