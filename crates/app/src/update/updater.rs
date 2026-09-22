//! `Updater` — the GPUI-side owner of self-update state.
//!
//! Mirrors Zed's `AutoUpdater` entity + `GlobalAutoUpdate` wrapper and
//! daruda's own `SettingsStore` global pattern: a single `Entity<Updater>`
//! registered behind a `GlobalUpdater` marker so any view can resolve the
//! live handle via [`Updater::get`] and drive it with `entity.update(...)`.
//!
//! The three blocking `daruda_update` calls (`check_latest`, `download_asset`,
//! `install_dmg`) run on `cx.background_executor()`; every status transition
//! flips back onto the foreground inside `this.update(cx, ...)` so `cx.notify`
//! fires on the GPUI main thread. `daruda_update` stays GPUI-free — this file
//! is the only place update state touches an `App`.

use std::path::{Path, PathBuf};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_update::{ReleaseInfo, UpdateError};
use gpui::{App, AppContext as _, Context, Entity, Global};

/// The self-update lifecycle as a single enum so invalid combinations
/// (e.g. "downloading" while also holding a ready-to-restart path) are
/// unrepresentable.
#[derive(Clone, Debug)]
pub enum AutoUpdateStatus {
    /// No check has run, or the last flow completed and reset.
    Idle,
    /// A `check_latest` request is in flight.
    Checking,
    /// The latest release is not newer than the running build.
    UpToDate,
    /// A newer release is available and ready to download.
    Available(ReleaseInfo),
    /// The DMG is being downloaded.
    Downloading,
    /// The downloaded DMG is being mounted and swapped over the bundle.
    Installing,
    /// The swap succeeded; holds what to relaunch into.
    ReadyToRestart(InstallTarget),
    /// A step failed; carries the `UpdateError` `Display` text.
    Errored(String),
}

/// GPUI-side owner of update state. One live entity per process,
/// reachable through [`GlobalUpdater`]. Fields are driven by the async
/// flow and surfaced through the accessors below (About section + the
/// workspace toast).
pub struct Updater {
    status: AutoUpdateStatus,
    /// Parsed `CARGO_PKG_VERSION` of the running build.
    current: semver::Version,
    /// The running `.app` bundle path, `Some` only when launched from a
    /// real bundle. `None` under `cargo run` — the install gate keys off
    /// this so a dev build never tries to swap a bundle that isn't there.
    target: Option<InstallTarget>,
}

/// Newtype marker so the `Global` impl lives in the app crate. Holds the
/// live entity, or `None` if [`Updater::init`] skipped registration
/// (unparseable version). Read through [`Updater::get`].
struct GlobalUpdater(Option<Entity<Updater>>);

impl Global for GlobalUpdater {}

impl Updater {
    /// Idempotent bootstrap. Parses the running build's version, resolves
    /// what an install would replace (the install gate), creates the entity,
    /// and registers it as the `GlobalUpdater`. A `has_global` guard
    /// keeps a second call (test fixtures + production entry) from clobbering
    /// an already-registered global.
    ///
    /// If `CARGO_PKG_VERSION` somehow fails to parse, logs and skips
    /// registration rather than panicking — the update UI then simply never
    /// resolves a handle.
    pub fn init(cx: &mut App) {
        if cx.has_global::<GlobalUpdater>() {
            return;
        }

        let current = match semver::Version::parse(env!("CARGO_PKG_VERSION")) {
            Ok(v) => v,
            Err(e) => {
                LogWriter::log(
                    ErrorReport::new("Auto-update disabled: version parse failed")
                        .message(e.to_string())
                        .severity(ErrorSeverity::Warning)
                        .at(file!(), line!())
                        .build(),
                );
                return;
            }
        };

        let target = cx.app_path().ok().and_then(|exe| InstallTarget::of(&exe));

        let entity = cx.new(|_| Updater {
            status: AutoUpdateStatus::Idle,
            current,
            target,
        });
        cx.set_global(GlobalUpdater(Some(entity)));
    }

    /// The live entity, if [`init`](Self::init) registered one.
    pub fn get(cx: &mut App) -> Option<Entity<Updater>> {
        cx.try_global::<GlobalUpdater>().and_then(|g| g.0.clone())
    }

    /// The current lifecycle status.
    pub fn status(&self) -> &AutoUpdateStatus {
        &self.status
    }

    /// True when the running build sits somewhere this can replace in place.
    pub fn can_install(&self) -> bool {
        self.target.is_some()
    }

    /// Clear what an earlier update left behind, if this build is the kind
    /// that leaves anything.
    pub fn sweep(&self) {
        if let Some(target) = &self.target {
            target.sweep();
        }
    }

    /// Kick off a background `check_latest`. No-op while a flow is already
    /// in flight.
    pub fn check(&mut self, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }
        self.status = AutoUpdateStatus::Checking;
        cx.notify();

        let current = self.current.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { daruda_update::check_latest(&current) })
                .await;
            // SILENT-OK: app shutting down mid-update; the entity update is moot
            let _ = this.update(cx, |updater, cx| updater.apply_check_result(result, cx));
        })
        .detach();
    }

    /// Download the available release's DMG and swap it over the running
    /// bundle. Only proceeds when the status is `Available` *and* we know
    /// our own `.app` bundle path (the install gate); otherwise a no-op.
    pub fn download_and_install(&mut self, cx: &mut Context<Self>) {
        let info = match &self.status {
            AutoUpdateStatus::Available(info) => info.clone(),
            _ => return,
        };
        let Some(target) = self.target.clone() else {
            return;
        };

        self.status = AutoUpdateStatus::Downloading;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let dest = std::env::temp_dir().join(format!("daruda-update-{}.dmg", info.version));
            let url = info.asset_url.clone();
            let dest_for_dl = dest.clone();

            // hop A — download to a temp path on the background executor.
            let downloaded = cx
                .background_executor()
                .spawn(async move {
                    daruda_update::download_asset(&url, &dest_for_dl).map(|()| dest_for_dl)
                })
                .await;

            let dmg = match downloaded {
                Ok(dmg) => dmg,
                Err(e) => {
                    // SILENT-OK: app shutting down mid-update; the entity update is moot
                    let _ = this.update(cx, |updater, cx| updater.fail(&e, cx));
                    return;
                }
            };

            // foreground — flip to Installing before the second hop.
            // SILENT-OK: app shutting down mid-update; the entity update is moot
            let _ = this.update(cx, |updater, cx| {
                updater.status = AutoUpdateStatus::Installing;
                cx.notify();
            });

            // hop B — mount + swap the bundle on the background executor.
            let target_for_install = target.clone();
            let package = dmg.clone();
            let installed = cx
                .background_executor()
                .spawn(async move { target_for_install.install(&package) })
                .await;

            // Best-effort cleanup of the downloaded image on either outcome —
            // a failed install must not leave the multi-MB DMG behind.
            let _ = std::fs::remove_file(&dmg);
            // SILENT-OK: app shutting down mid-update; the entity update is moot
            let _ = this.update(cx, |updater, cx| match installed {
                Ok(()) => {
                    updater.status = AutoUpdateStatus::ReadyToRestart(target);
                    cx.notify();
                }
                Err(e) => updater.fail(&e, cx),
            });
        })
        .detach();
    }

    /// Relaunch into the swapped bundle and quit this process. Only acts on
    /// a `ReadyToRestart` status. `relaunch` is non-blocking (it spawns a
    /// detached shell that waits for this pid to exit, then reopens), so it
    /// runs on the main thread directly.
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        let AutoUpdateStatus::ReadyToRestart(target) = &self.status else {
            return;
        };
        let target = target.clone();
        match target.relaunch() {
            Ok(()) => cx.quit(),
            Err(e) => self.fail(&e, cx),
        }
    }

    /// True while a background hop is running. Guards re-entrant [`check`]
    /// and (indirectly) any UI double-trigger.
    fn is_busy(&self) -> bool {
        matches!(
            self.status,
            AutoUpdateStatus::Checking
                | AutoUpdateStatus::Downloading
                | AutoUpdateStatus::Installing
        )
    }

    /// Foreground continuation for [`check`]: map the `check_latest` result
    /// onto a status and notify.
    fn apply_check_result(
        &mut self,
        result: Result<Option<ReleaseInfo>, UpdateError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(Some(info)) => self.status = AutoUpdateStatus::Available(info),
            Ok(None) => self.status = AutoUpdateStatus::UpToDate,
            Err(e) => {
                self.fail(&e, cx);
                return;
            }
        }
        cx.notify();
    }

    /// Log the failure to the NDJSON pipeline and move to `Errored`.
    /// Toasting is the Workspace's job (a later task); this entity only
    /// logs and records status.
    fn fail(&mut self, err: &UpdateError, cx: &mut Context<Self>) {
        LogWriter::log(
            ErrorReport::new("Update failed")
                .from_error(err)
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .build(),
        );
        self.status = AutoUpdateStatus::Errored(err.to_string());
        cx.notify();
    }
}

/// `<…>/target/{debug,release}` — where cargo puts a build, and the one
/// place a portable install never is.
fn is_cargo_output(dir: &Path) -> bool {
    matches!(
        dir.file_name().and_then(|name| name.to_str()),
        Some("debug" | "release")
    ) && dir
        .parent()
        .and_then(|parent| parent.file_name())
        .is_some_and(|name| name == "target")
}

/// Where the running build lives, and therefore how it is replaced.
///
/// One value rather than a `cfg` at each step: macOS ships a bundle rsync
/// copies into, Windows a directory whose files are renamed aside. Both
/// arms are reachable from either host, so the Windows answer is tested
/// without being run on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallTarget {
    /// The `.app` the running executable sits inside.
    Bundle(PathBuf),
    /// The directory a portable archive was extracted to.
    Directory(PathBuf),
}

impl InstallTarget {
    /// What `exe` can be replaced as, or `None` for a build that is not an
    /// install at all — `cargo run`, whose parent is a `target/` directory.
    fn of(exe: &Path) -> Option<Self> {
        Self::for_host(exe, cfg!(windows))
    }

    /// [`Self::of`] with the host as a value.
    fn for_host(exe: &Path, windows: bool) -> Option<Self> {
        if windows {
            // A portable install is the directory holding the executable.
            // Everything but a cargo build looks like one, and that directory
            // is writable — so the swap's own check would not save a developer
            // from having a release dropped over `target\\debug`.
            return exe
                .parent()
                .filter(|dir| !is_cargo_output(dir))
                .map(|dir| Self::Directory(dir.to_path_buf()));
        }
        exe.ancestors()
            .find(|path| path.extension().is_some_and(|ext| ext == "app"))
            .map(|bundle| Self::Bundle(bundle.to_path_buf()))
    }

    fn install(&self, package: &Path) -> Result<(), UpdateError> {
        match self {
            Self::Bundle(bundle) => daruda_update::install_dmg(package, bundle),
            Self::Directory(root) => daruda_update::install_zip(package, root),
        }
    }

    /// Remove what an earlier swap left behind. A bundle has none: `rsync`
    /// replaces its contents outright, and the path `app_path` hands back
    /// there is `/Applications`, which is nobody's install root.
    pub fn sweep(&self) {
        if let Self::Directory(root) = self {
            daruda_update::sweep_aside(root);
        }
    }

    fn relaunch(&self) -> Result<(), UpdateError> {
        match self {
            Self::Bundle(bundle) => daruda_update::relaunch(bundle),
            Self::Directory(root) => daruda_update::relaunch_from(root),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn updater_with(status: AutoUpdateStatus) -> Updater {
        Updater {
            status,
            current: semver::Version::new(0, 2, 0),
            target: None,
        }
    }

    #[test]
    fn is_busy_true_for_in_flight_states() {
        for status in [
            AutoUpdateStatus::Checking,
            AutoUpdateStatus::Downloading,
            AutoUpdateStatus::Installing,
        ] {
            assert!(
                updater_with(status.clone()).is_busy(),
                "{status:?} must count as busy"
            );
        }
    }

    #[test]
    fn is_busy_false_for_settled_states() {
        let info = ReleaseInfo {
            version: semver::Version::new(0, 3, 0),
            tag: "v0.3.0".to_string(),
            asset_url: "https://github.com/x.dmg".to_string(),
            notes: String::new(),
        };
        for status in [
            AutoUpdateStatus::Idle,
            AutoUpdateStatus::UpToDate,
            AutoUpdateStatus::Available(info),
            AutoUpdateStatus::ReadyToRestart(InstallTarget::Bundle(PathBuf::from(
                "/Applications/daruda.app",
            ))),
            AutoUpdateStatus::Errored("boom".to_string()),
        ] {
            assert!(
                !updater_with(status.clone()).is_busy(),
                "{status:?} must not count as busy"
            );
        }
    }

    #[test]
    fn a_bundled_mac_build_is_replaced_as_its_bundle() {
        let exe = Path::new("/Applications/daruda.app/Contents/MacOS/daruda");
        assert_eq!(
            InstallTarget::for_host(exe, false),
            Some(InstallTarget::Bundle(PathBuf::from(
                "/Applications/daruda.app"
            )))
        );
    }

    #[test]
    fn a_mac_dev_build_is_not_an_install() {
        let exe = Path::new("/Users/dev/daruda/target/debug/daruda");
        assert_eq!(InstallTarget::for_host(exe, false), None);
    }

    /// A portable install has no marker in its path, so the directory holding
    /// the executable is the answer. Forward slashes because `Path` splits on
    /// the *host's* separator — a backslash is an ordinary character here on
    /// macOS, and Windows takes either. Only the branch is under test.
    #[test]
    fn a_windows_build_is_replaced_as_the_directory_it_sits_in() {
        let exe = Path::new("C:/Users/me/daruda-0.3.0-windows-x86_64/daruda.exe");
        assert_eq!(
            InstallTarget::for_host(exe, true),
            Some(InstallTarget::Directory(PathBuf::from(
                "C:/Users/me/daruda-0.3.0-windows-x86_64"
            )))
        );
    }

    /// A developer running `cargo run` on Windows must not have a release
    /// dropped over the build directory — which is writable, so the swap's
    /// own check would not have stopped it.
    #[test]
    fn a_windows_cargo_build_is_not_an_install() {
        for exe in [
            "C:/src/daruda/target/debug/daruda.exe",
            "C:/src/daruda/target/release/daruda.exe",
        ] {
            assert_eq!(InstallTarget::for_host(Path::new(exe), true), None, "{exe}");
        }
    }

    /// The guard is narrow on purpose: a real install may well sit in a
    /// directory called `release`, just not one under `target`.
    #[test]
    fn a_directory_named_release_outside_target_is_still_an_install() {
        let exe = Path::new("C:/Users/me/release/daruda.exe");

        assert_eq!(
            InstallTarget::for_host(exe, true),
            Some(InstallTarget::Directory(PathBuf::from(
                "C:/Users/me/release"
            )))
        );
    }

    /// The same path answers differently per host, which is the whole reason
    /// the decision is one value rather than a `cfg` at each step.
    #[test]
    fn the_two_hosts_do_not_answer_alike() {
        let exe = Path::new("/Applications/daruda.app/Contents/MacOS/daruda");

        assert_ne!(
            InstallTarget::for_host(exe, true),
            InstallTarget::for_host(exe, false)
        );
    }

    #[test]
    fn can_install_tracks_the_target() {
        let mut updater = updater_with(AutoUpdateStatus::Idle);
        assert!(!updater.can_install());
        updater.target = Some(InstallTarget::Bundle(PathBuf::from(
            "/Applications/daruda.app",
        )));
        assert!(updater.can_install());
    }
}
