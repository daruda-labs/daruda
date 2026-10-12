//! What the running build is, and therefore whether and how it is replaced.
//!
//! Decided once from the executable's path and the host as values, so every
//! host's answer is tested from any one of them.

use std::path::{Path, PathBuf};

use daruda_store::observability::error_report::ErrorReport;
use daruda_store::observability::log_writer::LogWriter;
use daruda_update::UpdateError;

/// The platforms whose builds differ in how they are installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Host {
    MacOs,
    Windows,
    Linux,
}

impl Host {
    /// The platform this binary was built for, or `None` where daruda ships
    /// no package at all.
    pub(super) fn current() -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Self::MacOs)
        } else if cfg!(windows) {
            Some(Self::Windows)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }
}

/// Whether this process may replace its own build, and if not, why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallGate {
    /// Replaced in place by this process.
    Target(InstallTarget),
    /// A Linux package: the package manager, not daruda, owns its files.
    ManualPackage,
    /// A cargo build, or a host daruda ships no package for.
    Unpackaged,
}

impl InstallGate {
    pub(super) fn of(exe: &Path) -> Self {
        Self::for_host(exe, Host::current())
    }

    /// [`Self::of`] with the host as a value.
    pub(super) fn for_host(exe: &Path, host: Option<Host>) -> Self {
        let Some(host) = host else {
            return Self::Unpackaged;
        };
        match host {
            Host::MacOs => exe
                .ancestors()
                .find(|path| path.extension().is_some_and(|ext| ext == "app"))
                .map_or(Self::Unpackaged, |bundle| {
                    Self::Target(InstallTarget::Bundle(bundle.to_path_buf()))
                }),
            // A cargo build directory is writable, so the swap's own check
            // would not stop a release being dropped over `target\debug`.
            Host::Windows => match exe.parent().filter(|dir| !is_cargo_output(dir)) {
                Some(dir) => Self::Target(match super::InstallerTarget::discover(dir) {
                    Some(installer) => InstallTarget::Installer(installer),
                    None => InstallTarget::Directory(dir.to_path_buf()),
                }),
                None => Self::Unpackaged,
            },
            Host::Linux => match exe.parent() {
                Some(dir) if is_cargo_output(dir) => Self::Unpackaged,
                _ => Self::ManualPackage,
            },
        }
    }

    pub(super) fn target(&self) -> Option<&InstallTarget> {
        match self {
            Self::Target(target) => Some(target),
            Self::ManualPackage | Self::Unpackaged => None,
        }
    }
}

/// `<…>/target/{debug,release}` — where cargo puts a build, and the one
/// place an installed build never is.
fn is_cargo_output(dir: &Path) -> bool {
    matches!(
        dir.file_name().and_then(|name| name.to_str()),
        Some("debug" | "release")
    ) && dir
        .parent()
        .and_then(|parent| parent.file_name())
        .is_some_and(|name| name == "target")
}

/// How a replaceable build is replaced: macOS rsyncs into its bundle,
/// a portable Windows install renames its files aside, NSIS runs itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallTarget {
    /// The `.app` the running executable sits inside.
    Bundle(PathBuf),
    /// The directory a portable archive was extracted to.
    Directory(PathBuf),
    /// NSIS owns file manifests and Windows registration for this deployment.
    Installer(super::InstallerTarget),
}

impl InstallTarget {
    pub(super) fn asset_suffix(&self) -> Option<&'static str> {
        match self {
            Self::Installer(_) => Some(super::installer::ASSET_SUFFIX),
            _ => daruda_update::asset_suffix(),
        }
    }

    pub(crate) fn uses_installer(&self) -> bool {
        matches!(self, Self::Installer(_))
    }

    pub(super) fn install(&self, package: &Path) -> Result<Self, UpdateError> {
        match self {
            Self::Installer(installer) => installer.prepare(package).map(Self::Installer),
            Self::Bundle(bundle) => {
                daruda_update::install_dmg(package, bundle).map(|()| self.clone())
            }
            Self::Directory(root) => {
                daruda_update::install_zip_verified(package, root, &|bundle| {
                    crate::platform::authenticode::verify_update(
                        &root.join("daruda.exe"),
                        &bundle.join("daruda.exe"),
                    )
                    .map_err(|error| UpdateError::Sync(error.to_string()))
                })
                .map(|()| self.clone())
            }
        }
    }

    /// Remove what an earlier swap left behind. A bundle has none: `rsync`
    /// replaces its contents outright, and the path `app_path` hands back
    /// there is `/Applications`, which is nobody's install root.
    pub fn sweep(&self) {
        if let Self::Directory(root) = self {
            match daruda_update::recover_update(root) {
                Ok(()) => daruda_update::sweep_aside(root),
                Err(error) => LogWriter::log(
                    ErrorReport::new("Interrupted update recovery failed")
                        .message(error.to_string())
                        .at(file!(), line!())
                        .dedup("update.recovery.failed")
                        .build(),
                ),
            }
        }
    }

    pub(super) fn relaunch(&self) -> Result<(), UpdateError> {
        match self {
            Self::Bundle(bundle) => daruda_update::relaunch(bundle),
            Self::Directory(root) => daruda_update::relaunch_from(root),
            Self::Installer(installer) => installer.relaunch(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(exe: &str, host: Host) -> InstallGate {
        InstallGate::for_host(Path::new(exe), Some(host))
    }

    #[test]
    fn a_bundled_mac_build_is_replaced_as_its_bundle() {
        assert_eq!(
            target(
                "/Applications/daruda.app/Contents/MacOS/daruda",
                Host::MacOs
            ),
            InstallGate::Target(InstallTarget::Bundle(PathBuf::from(
                "/Applications/daruda.app"
            )))
        );
    }

    #[test]
    fn a_mac_dev_build_is_not_an_install() {
        assert_eq!(
            target("/Users/dev/daruda/target/debug/daruda", Host::MacOs),
            InstallGate::Unpackaged
        );
    }

    /// A portable install has no marker in its path, so the directory holding
    /// the executable is the answer. Forward slashes because `Path` splits on
    /// the *host's* separator — a backslash is an ordinary character here on
    /// macOS, and Windows takes either. Only the branch is under test.
    #[test]
    fn a_windows_build_is_replaced_as_the_directory_it_sits_in() {
        assert_eq!(
            target(
                "C:/Users/me/daruda-0.3.0-windows-x86_64/daruda.exe",
                Host::Windows
            ),
            InstallGate::Target(InstallTarget::Directory(PathBuf::from(
                "C:/Users/me/daruda-0.3.0-windows-x86_64"
            )))
        );
    }

    #[test]
    fn a_windows_cargo_build_is_not_an_install() {
        for exe in [
            "C:/src/daruda/target/debug/daruda.exe",
            "C:/src/daruda/target/release/daruda.exe",
        ] {
            assert_eq!(target(exe, Host::Windows), InstallGate::Unpackaged, "{exe}");
        }
    }

    /// The guard is narrow on purpose: a real install may well sit in a
    /// directory called `release`, just not one under `target`.
    #[test]
    fn a_directory_named_release_outside_target_is_still_an_install() {
        assert_eq!(
            target("C:/Users/me/release/daruda.exe", Host::Windows),
            InstallGate::Target(InstallTarget::Directory(PathBuf::from(
                "C:/Users/me/release"
            )))
        );
    }

    #[test]
    fn managed_windows_installations_use_setup_instead_of_zip() {
        let root = tempfile::tempdir().unwrap();
        let exe = root.path().join("daruda.exe");
        std::fs::write(root.path().join("uninstall.exe"), "installer owned").unwrap();
        let gate = InstallGate::for_host(&exe, Some(Host::Windows));
        let target = gate.target().unwrap();
        assert!(target.uses_installer());
        assert_eq!(
            target.asset_suffix(),
            Some(super::super::installer::ASSET_SUFFIX)
        );
    }

    /// Both the deb's `/usr/bin` and an extracted tar.gz are announced, not
    /// replaced: the first belongs to dpkg, and neither has a swap path yet.
    #[test]
    fn a_linux_install_is_left_to_its_package() {
        for exe in [
            "/usr/bin/daruda",
            "/home/me/daruda-0.3.0-linux-x86_64/daruda",
        ] {
            assert_eq!(
                target(exe, Host::Linux),
                InstallGate::ManualPackage,
                "{exe}"
            );
        }
    }

    #[test]
    fn a_linux_cargo_build_is_not_a_package() {
        assert_eq!(
            target("/home/me/daruda/target/debug/daruda", Host::Linux),
            InstallGate::Unpackaged
        );
    }

    /// A Linux path inside a directory named like a bundle is still a Linux
    /// install; the macOS rule must not leak across hosts.
    #[test]
    fn each_host_applies_only_its_own_rule() {
        let exe = "/opt/daruda.app/daruda";
        assert!(matches!(target(exe, Host::MacOs), InstallGate::Target(_)));
        assert_eq!(target(exe, Host::Linux), InstallGate::ManualPackage);
    }

    #[test]
    fn a_host_without_packages_is_unpackaged() {
        assert_eq!(
            InstallGate::for_host(Path::new("/usr/local/bin/daruda"), None),
            InstallGate::Unpackaged
        );
    }
}
