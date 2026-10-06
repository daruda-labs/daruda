//! File-system watcher for layered `config.toml` live reload.
//!
//! Watches the config root recursively so user-global and per-project files
//! share one debounced signal path. [`ConfigWatcherHandle`] owns the live
//! watcher; dropping it disconnects the debounce thread cleanly.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

/// Minimum interval between successive reloads. File-save events
/// often arrive in bursts (write + rename + chmod); debouncing
/// prevents flicker from partial writes.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(200);

/// RAII handle: keep it alive while reload notifications are needed.
pub struct ConfigWatcherHandle {
    debounced_rx: mpsc::Receiver<()>,
    /// Dropping this stops the watch and lets the debounce thread exit.
    _watcher: crate::dir_watch::DirWatcher,
}

impl ConfigWatcherHandle {
    /// Non-blocking poll for a debounced reload signal.
    pub fn try_recv(&self) -> Result<(), mpsc::TryRecvError> {
        self.debounced_rx.try_recv()
    }
}

/// Start watching all `config.toml` files under the config root.
/// On an FSEvents rescan, emit one reload so layered config is re-read.
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
    // Recursive so project overlays share the user-global watcher.
    let anchors = [watch_dir];
    let (raw_rx, watcher) =
        crate::dir_watch::spawn_dir_watcher(&anchors, RecursiveMode::Recursive, classify, || {
            vec![()]
        });

    // Collapse a save burst into one reload signal.
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
        // Per-process dir, like the git/boot fixtures: a fixed path makes the
        // test order-dependent once anything else in the run touches it.
        let dir =
            std::env::temp_dir().join(format!("daruda_config_reload_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "[font]").unwrap();
        writeln!(f, "size = 18.0").unwrap();
        drop(f);

        let cfg = daruda_config::Config::load_from(&path);
        assert_eq!(cfg.font.terminal.size, 18.0);

        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "[font]").unwrap();
        writeln!(f, "size = 22.0").unwrap();
        drop(f);

        let cfg2 = daruda_config::Config::load_from(&path);
        assert_eq!(cfg2.font.terminal.size, 22.0);

        let _ = std::fs::remove_dir_all(dir);
    }
}
