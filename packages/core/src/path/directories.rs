//! OS directory discovery, without application names or filesystem writes.

use std::path::{Path, PathBuf};

/// A snapshot of this user's OS-provided storage locations.
/// Application naming and profile isolation belong to the caller.
pub struct UserDirectories {
    config: Option<PathBuf>,
    home: Option<PathBuf>,
    logs: Option<ApplicationBase>,
    state: Option<ApplicationBase>,
}

struct ApplicationBase {
    root: PathBuf,
    suffix: &'static str,
}

#[derive(Clone, Copy)]
enum Platform {
    Windows,
    Mac,
    Xdg,
}

impl UserDirectories {
    /// Discover directories once. Does not create or inspect files.
    pub fn discover() -> Self {
        // This capability is the sole OS config-directory boundary;
        // application callers must apply profile policy in the store layer.
        #[allow(clippy::disallowed_methods)]
        let config = dirs::config_dir();
        let home = dirs::home_dir();
        let platform = if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Xdg
        };
        #[allow(clippy::disallowed_methods)] // OS discovery boundary; no app policy here.
        let local = dirs::data_local_dir();
        #[allow(clippy::disallowed_methods)] // OS discovery boundary.
        let state_root = dirs::state_dir();
        let state =
            ApplicationBase::state(platform, home.as_deref(), local.clone(), state_root.clone());
        let logs = ApplicationBase::resolve(platform, home.as_deref(), local, state_root);
        Self {
            config,
            home,
            logs,
            state,
        }
    }

    /// Base for application configuration, before application naming.
    pub fn config_root(&self) -> Option<&Path> {
        self.config.as_deref()
    }

    /// User home, for reading compatibility artifacts owned by older versions.
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    /// Native log directory for an already validated application directory name.
    pub fn logs_for(&self, application: &str) -> Option<PathBuf> {
        self.logs
            .as_ref()
            .map(|base| base.for_application(application))
    }

    /// Native base for persistent machine-specific state of an application.
    pub fn state_for(&self, application: &str) -> Option<PathBuf> {
        self.state
            .as_ref()
            .map(|base| base.for_application(application))
    }
}

impl ApplicationBase {
    fn state(
        platform: Platform,
        home: Option<&Path>,
        local: Option<PathBuf>,
        state: Option<PathBuf>,
    ) -> Option<Self> {
        match platform {
            Platform::Windows => local.map(|root| Self {
                root,
                suffix: "state",
            }),
            Platform::Mac => home.map(|home| Self {
                root: home.join("Library").join("Application Support"),
                suffix: "state",
            }),
            Platform::Xdg => state.map(|root| Self { root, suffix: "" }),
        }
    }
    fn resolve(
        platform: Platform,
        home: Option<&Path>,
        local: Option<PathBuf>,
        state: Option<PathBuf>,
    ) -> Option<Self> {
        match platform {
            Platform::Windows => local.map(|root| Self {
                root,
                suffix: "logs",
            }),
            Platform::Mac => home.map(|home| Self {
                root: home.join("Library").join("Logs"),
                suffix: "",
            }),
            Platform::Xdg => state.map(|root| Self {
                root,
                suffix: "logs",
            }),
        }
    }

    fn for_application(&self, application: &str) -> PathBuf {
        let directory = self.root.join(application);
        if self.suffix.is_empty() {
            directory
        } else {
            directory.join(self.suffix)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_log_layouts_do_not_depend_on_the_build_host() {
        for (platform, expected) in [
            (Platform::Windows, "local/daruda-debug/logs"),
            (Platform::Mac, "home/Library/Logs/daruda-debug"),
            (Platform::Xdg, "custom-state/daruda-debug/logs"),
        ] {
            let base = ApplicationBase::resolve(
                platform,
                Some(Path::new("home")),
                Some("local".into()),
                Some("custom-state".into()),
            )
            .unwrap();
            assert_eq!(
                base.for_application("daruda-debug"),
                PathBuf::from(expected)
            );
        }
    }

    #[test]
    fn unavailable_directories_remain_unresolved() {
        for platform in [Platform::Windows, Platform::Mac, Platform::Xdg] {
            assert!(ApplicationBase::resolve(platform, None, None, None).is_none());
        }
    }

    #[test]
    fn state_directories_follow_each_platform_policy() {
        for (platform, expected) in [
            (Platform::Windows, "local/daruda-preview/state"),
            (
                Platform::Mac,
                "home/Library/Application Support/daruda-preview/state",
            ),
            (Platform::Xdg, "custom-state/daruda-preview"),
        ] {
            let base = ApplicationBase::state(
                platform,
                Some(Path::new("home")),
                Some("local".into()),
                Some("custom-state".into()),
            )
            .unwrap();
            assert_eq!(
                base.for_application("daruda-preview"),
                PathBuf::from(expected)
            );
        }
    }
}
