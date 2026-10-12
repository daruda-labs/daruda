//! `Updater` — the GPUI-side owner of self-update state.
//!
//! Mirrors Zed's `AutoUpdater` entity + `GlobalAutoUpdate` wrapper and
//! daruda's own `SettingsStore` global pattern: a single `Entity<Updater>`
//! registered behind a `GlobalUpdater` marker so any view can resolve the
//! live handle via [`Updater::get`] and drive it with `entity.update(...)`.
//!
//! The three blocking `daruda_update` calls (`check_latest`, `download_verified`,
//! `install_dmg`) run on `cx.background_executor()`; every status transition
//! flips back onto the foreground inside `this.update(cx, ...)` so `cx.notify`
//! fires on the GPUI main thread. `daruda_update` stays GPUI-free — this file
//! is the only place update state touches an `App`.

use std::path::Path;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_update::{ReleaseInfo, UpdateError};
use gpui::{App, AppContext as _, Context, Entity, Global};

use super::{InstallGate, InstallTarget};

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
    /// Whether this build can be replaced in place, decided once at init so
    /// a `cargo run` build never tries to swap files that are not an install.
    gate: InstallGate,
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

        let gate = cx
            .app_path()
            .map_or(InstallGate::Unpackaged, |exe| InstallGate::of(&exe));

        let entity = cx.new(|_| Updater {
            status: AutoUpdateStatus::Idle,
            current,
            gate,
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

    /// Whether the running build can be replaced in place, and if not, why.
    pub fn install_gate(&self) -> &InstallGate {
        &self.gate
    }

    /// Clear what an earlier update left behind, if this build is the kind
    /// that leaves anything.
    pub fn sweep(&self) {
        if let Some(target) = self.gate.target() {
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
        let suffix = self.gate.target().and_then(InstallTarget::asset_suffix);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { check_latest(&current, suffix) })
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
        let Some(target) = self.gate.target().cloned() else {
            return;
        };

        self.status = AutoUpdateStatus::Downloading;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let temporary = match tempfile::tempdir() {
                Ok(directory) => directory,
                Err(error) => {
                    // SILENT-OK: app shutting down mid-update; the entity update is moot
                    let _ = this.update(cx, |updater, cx| {
                        updater.fail(&UpdateError::Io(error.to_string()), cx)
                    });
                    return;
                }
            };
            let dest = temporary.path().join("package");
            let release = info.clone();
            let dest_for_dl = dest.clone();

            // hop A — download to a temp path on the background executor.
            let downloaded = cx
                .background_executor()
                .spawn(
                    async move { download_package(&release, &dest_for_dl).map(|()| dest_for_dl) },
                )
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
                Ok(target) => {
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

/// Tests must never reach GitHub: under test every check answers "up to date".
fn check_latest(
    current: &semver::Version,
    suffix: Option<&'static str>,
) -> Result<Option<ReleaseInfo>, UpdateError> {
    if cfg!(test) {
        return Ok(None);
    }
    match suffix {
        Some(suffix) => daruda_update::check::check_latest_with_suffix(current, suffix),
        None => daruda_update::check_latest(current),
    }
}

/// The release's package, verified against its published checksum. Under
/// test no release is ever offered, so a download has nothing to fetch.
fn download_package(release: &ReleaseInfo, dest: &Path) -> Result<(), UpdateError> {
    if cfg!(test) {
        return Err(UpdateError::Http(OFFLINE_DOWNLOAD.to_owned()));
    }
    daruda_update::download_verified(release, dest)
}

const OFFLINE_DOWNLOAD: &str = "network is disabled in tests";

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn updater_with(status: AutoUpdateStatus) -> Updater {
        Updater {
            status,
            current: semver::Version::new(0, 2, 0),
            gate: InstallGate::Unpackaged,
        }
    }

    #[test]
    fn network_calls_are_stubbed_under_test() {
        let current = semver::Version::new(0, 2, 0);
        assert!(matches!(check_latest(&current, None), Ok(None)));
        let release = ReleaseInfo {
            version: semver::Version::new(0, 3, 0),
            tag: "v0.3.0".to_string(),
            asset_url: "https://github.com/x.dmg".to_string(),
            asset_name: "x.dmg".to_string(),
            checksums_url: "https://github.com/SHA256SUMS.txt".to_string(),
            notes: String::new(),
        };
        let dest = std::env::temp_dir().join("daruda-update-offline.dmg");
        assert!(matches!(
            download_package(&release, &dest),
            Err(UpdateError::Http(_))
        ));
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
            asset_name: "x.dmg".to_string(),
            checksums_url: "https://github.com/SHA256SUMS.txt".to_string(),
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
}
