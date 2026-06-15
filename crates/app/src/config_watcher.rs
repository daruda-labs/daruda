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

/// Minimum interval between successive reloads. File-save events
/// often arrive in bursts (write + rename + chmod); debouncing
/// prevents flicker from partial writes.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(200);

/// RAII handle returned by [`spawn_config_watcher`]. Owns the
/// [`crate::dir_watch::DirWatcher`] so the kernel-side subscription stays
/// attached for the handle's lifetime — dropping the handle ends the watch,
/// which disconnects the raw channel and lets the debounce thread exit
/// cleanly.
pub struct ConfigWatcherHandle {
    debounced_rx: mpsc::Receiver<()>,
    /// Dropping this stops the watch; the debounce thread then sees its raw
    /// `Receiver` disconnect and exits. Declared after `debounced_rx` so it is
    /// dropped first, but ordering is not load-bearing — either drop ends the
    /// thread.
    _watcher: crate::dir_watch::DirWatcher,
}

impl ConfigWatcherHandle {
    /// Non-blocking poll for a debounced reload signal.
    pub fn try_recv(&self) -> Result<(), mpsc::TryRecvError> {
        self.debounced_rx.try_recv()
    }
}

/// Start watching the config file. Returns a [`ConfigWatcherHandle`]
/// the caller must keep alive for as long as it wants reload
/// notifications. Dropping the handle ends the kernel-side watch +
/// the debounce thread. On an FSEvents drop (sleep/wake) one reload signal is
/// emitted so the layered config is re-read.
pub fn spawn_config_watcher() -> ConfigWatcherHandle {
    use notify::RecursiveMode;

    let config_path = daruda_config::config_path();
    let watch_dir = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let file_name = config_path.file_name().map(|n| n.to_os_string());

    let classify = move |event: &notify::Event| {
        // Only trigger on a `config.toml`, ignoring sibling state files.
        let relevant = file_name.as_ref().is_none_or(|target| {
            event
                .paths
                .iter()
                .any(|p| p.file_name() == Some(target.as_os_str()))
        });
        if relevant { vec![()] } else { vec![] }
    };
    // Recursive so the same watcher picks up
    // `<config_dir>/daruda/projects/<...>/config.toml` writes alongside the
    // user-global file. `classify`'s filename filter gates which events fire.
    let anchors = [watch_dir];
    let (raw_rx, watcher) =
        crate::dir_watch::spawn_dir_watcher(&anchors, RecursiveMode::Recursive, classify, || {
            vec![()]
        });

    // Collapse a burst into one signal (shared helper; the thread exits when
    // the watcher is dropped and `raw_rx` disconnects).
    let debounced_rx = crate::dir_watch::debounce(raw_rx, DEBOUNCE);

    ConfigWatcherHandle {
        debounced_rx,
        _watcher: watcher,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

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
