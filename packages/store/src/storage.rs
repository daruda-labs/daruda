//! Private storage policy behind the public compatibility APIs.
//!
//! Resolution has no filesystem side effects. Workspace repositories migrate
//! during preparation; legacy configuration and shared lock roots stay stable.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use daruda_core::{path::directories::UserDirectories, process_env};

pub(crate) struct StorageLayout {
    data: PathBuf,
    shared: PathBuf,
    logs: Option<PathBuf>,
    legacy_logs: Option<PathBuf>,
    previous_native_logs: Option<PathBuf>,
    workspace_state: Option<PathBuf>,
}

impl StorageLayout {
    pub(crate) fn current() -> &'static Self {
        static LAYOUT: OnceLock<StorageLayout> = OnceLock::new();
        LAYOUT.get_or_init(|| {
            let directories = UserDirectories::discover();
            let override_root = override_root(process_env::DATA_DIR.read_os());
            let use_native_layout = override_root.is_none();
            let profile = crate::profile::active_profile();
            let application = application_name(profile);
            let config = directories.config_root().unwrap_or_else(|| Path::new("."));
            let layout = Self::resolve(
                config,
                directories.logs_for(&application),
                directories.state_for(&application),
                directories.home(),
                override_root,
                profile,
            );
            if use_native_layout {
                layout.with_profile_logs(directories.logs_for("daruda"), profile)
            } else {
                layout
            }
        })
    }

    fn resolve(
        config: &Path,
        native_logs: Option<PathBuf>,
        native_state: Option<PathBuf>,
        home: Option<&Path>,
        override_root: Option<PathBuf>,
        profile: &str,
    ) -> Self {
        match override_root {
            Some(root) => Self {
                logs: Some(root.join("logs")),
                workspace_state: Some(root.join("state").join("workspace")),
                data: root.clone(),
                shared: root,
                // An isolated run must never read the user's default logs.
                legacy_logs: None,
                previous_native_logs: None,
            },
            None => Self {
                data: config.join(application_name(profile)),
                shared: config.join("daruda"),
                logs: native_logs,
                workspace_state: native_state.map(|root| root.join("workspace")),
                legacy_logs: home.map(|home| home.join(".daruda/logs").join(profile)),
                previous_native_logs: None,
            },
        }
    }

    fn with_profile_logs(mut self, root: Option<PathBuf>, profile: &str) -> Self {
        self.previous_native_logs = self.logs.take();
        // A profile is one directory, never an absolute path or a traversal.
        self.logs = root.and_then(|root| {
            let mut components = Path::new(profile).components();
            match (components.next(), components.next()) {
                (Some(std::path::Component::Normal(_)), None) => Some(root.join(profile)),
                _ => None,
            }
        });
        self
    }

    pub(crate) fn data(&self) -> PathBuf {
        self.data.clone()
    }

    pub(crate) fn node_install(&self) -> PathBuf {
        self.shared.join("node")
    }

    pub(crate) fn workspace_state(&self) -> std::io::Result<PathBuf> {
        self.workspace_state.clone().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Cannot resolve workspace state storage",
            )
        })
    }

    pub(crate) fn flow_locks(&self) -> PathBuf {
        self.shared.join("flow-locks")
    }

    pub(crate) fn remote_locks(&self) -> PathBuf {
        self.shared.join("remote-locks")
    }

    pub(crate) fn logs(&self) -> Option<PathBuf> {
        self.logs.clone()
    }

    pub(crate) fn diagnostic_sources(&self) -> Vec<crate::observability::diagnostics::LogSource> {
        use crate::observability::diagnostics::LogSource;
        self.logs
            .iter()
            .cloned()
            .map(LogSource::Current)
            .chain(
                self.legacy_logs
                    .iter()
                    .cloned()
                    .map(LogSource::Compatibility),
            )
            .chain(
                self.previous_native_logs
                    .iter()
                    .cloned()
                    .map(LogSource::Compatibility),
            )
            .collect()
    }
}

fn application_name(profile: &str) -> String {
    match profile {
        crate::profile::RELEASE_PROFILE => "daruda".into(),
        other => format!("daruda-{other}"),
    }
}

fn override_root(value: Option<OsString>) -> Option<PathBuf> {
    let value = value?;
    let value = match value.to_str() {
        Some(value) => OsString::from(value.trim()),
        None => value,
    };
    if value.is_empty() {
        return None;
    }
    let root = PathBuf::from(value);
    // Bind relative overrides to the first lookup, not a later process cwd.
    Some(if root.is_relative() {
        daruda_core::path::absolute(&root).unwrap_or(root)
    } else {
        root
    })
}

#[cfg(test)]
mod tests;
