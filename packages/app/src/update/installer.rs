//! Installer-owned deployments retain registration and removal through NSIS.

use std::path::{Path, PathBuf};

use daruda_update::UpdateError;

pub(super) const ASSET_SUFFIX: &str = "-windows-x86_64-setup.exe";

/// A managed installation, with a verified installer only after preparation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallerTarget {
    root: PathBuf,
    state: InstallerState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum InstallerState {
    Installed,
    Prepared(PathBuf),
}

impl InstallerTarget {
    pub(super) fn discover(root: &Path) -> Option<Self> {
        // Older NSIS builds have no marker, but do own an uninstaller.
        (root.join("daruda-install.ini").is_file() || root.join("uninstall.exe").is_file()).then(
            || Self {
                root: root.into(),
                state: InstallerState::Installed,
            },
        )
    }

    /// Keep a private copy: the download is removed after preparation completes.
    pub(super) fn prepare(&self, verified_package: &Path) -> Result<Self, UpdateError> {
        let temporary = tempfile::Builder::new()
            .prefix("daruda-verified-update-")
            .suffix(".exe")
            .tempfile()
            .map_err(io_error)?;
        std::fs::copy(verified_package, temporary.path()).map_err(io_error)?;
        temporary.as_file().sync_all().map_err(io_error)?;
        let (file, path) = temporary.keep().map_err(|error| io_error(error.error))?;
        drop(file);
        // The staged installer stays available for retry and diagnosis in Temp.
        Ok(Self {
            root: self.root.clone(),
            state: InstallerState::Prepared(path),
        })
    }

    pub(super) fn relaunch(&self) -> Result<(), UpdateError> {
        match &self.state {
            InstallerState::Prepared(package) => {
                crate::platform::installer_update::launch(package, &self.root)
                    .map_err(|error| UpdateError::Io(error.to_string()))
            }
            InstallerState::Installed => {
                Err(UpdateError::Sync("Installer update is not prepared".into()))
            }
        }
    }
}

fn io_error(error: std::io::Error) -> UpdateError {
    UpdateError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_installation_and_preparation_preserve_owned_files() {
        let root = tempfile::tempdir().unwrap();
        assert!(InstallerTarget::discover(root.path()).is_none());
        std::fs::write(root.path().join("uninstall.exe"), "original uninstaller").unwrap();
        let installed = InstallerTarget::discover(root.path()).unwrap();
        assert!(installed.relaunch().is_err());
        let download = root.path().join("verified-setup.exe");
        std::fs::write(&download, "verified package bytes").unwrap();
        let prepared = installed.prepare(&download).unwrap();
        let InstallerState::Prepared(package) = prepared.state else {
            panic!("expected prepared installer")
        };
        assert_eq!(std::fs::read(&package).unwrap(), b"verified package bytes");
        assert_eq!(
            std::fs::read(root.path().join("uninstall.exe")).unwrap(),
            b"original uninstaller"
        );
        std::fs::remove_file(package).unwrap();
    }
}
