//! `Updater` — the GPUI-side owner of self-update state.
//!
//! Mirrors Zed's `AutoUpdater` entity + `GlobalAutoUpdate` wrapper and
//! daruda's own `SettingsStore` global pattern: a single `Entity<Updater>`
//! registered behind a `GlobalUpdater` marker so any view can resolve the
//! live handle via [`Updater::get`] and drive it with `entity.update(...)`.
//!
//! The three blocking `daruda_update` calls (`check_latest`, `download_dmg`,
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
    /// The swap succeeded; holds the `.app` bundle path to relaunch.
    ReadyToRestart(PathBuf),
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
    app_bundle: Option<PathBuf>,
}

/// Newtype marker so the `Global` impl lives in the app crate. Holds the
/// live entity, or `None` if [`Updater::init`] skipped registration
/// (unparseable version). Read through [`Updater::get`].
struct GlobalUpdater(Option<Entity<Updater>>);

impl Global for GlobalUpdater {}

impl Updater {
    /// Idempotent bootstrap. Parses the running build's version, detects
    /// whether we're inside a `.app` bundle (the install gate), creates the
    /// entity, and registers it as the `GlobalUpdater`. A `has_global` guard
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

        let app_bundle = cx.app_path().ok().and_then(|exe| app_bundle_from_exe(&exe));

        let entity = cx.new(|_| Updater {
            status: AutoUpdateStatus::Idle,
            current,
            app_bundle,
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

    /// True when running from a real `.app` bundle — the only case where an
    /// in-place install can succeed.
    pub fn can_install(&self) -> bool {
        self.app_bundle.is_some()
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
        let Some(app_bundle) = self.app_bundle.clone() else {
            return;
        };

        self.status = AutoUpdateStatus::Downloading;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let dest = std::env::temp_dir().join(format!("daruda-update-{}.dmg", info.version));
            let url = info.dmg_url.clone();
            let dest_for_dl = dest.clone();

            // hop A — download to a temp path on the background executor.
            let downloaded = cx
                .background_executor()
                .spawn(async move {
                    daruda_update::download_dmg(&url, &dest_for_dl).map(|()| dest_for_dl)
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
            let app_bundle_for_install = app_bundle.clone();
            let dmg_for_install = dmg.clone();
            let installed = cx
                .background_executor()
                .spawn(async move {
                    daruda_update::install_dmg(&dmg_for_install, &app_bundle_for_install)
                })
                .await;

            // Best-effort cleanup of the downloaded image on either outcome —
            // a failed install must not leave the multi-MB DMG behind.
            let _ = std::fs::remove_file(&dmg);
            // SILENT-OK: app shutting down mid-update; the entity update is moot
            let _ = this.update(cx, |updater, cx| match installed {
                Ok(()) => {
                    updater.status = AutoUpdateStatus::ReadyToRestart(app_bundle);
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
        let AutoUpdateStatus::ReadyToRestart(path) = &self.status else {
            return;
        };
        let path = path.clone();
        match daruda_update::relaunch(&path) {
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

/// Walk up from the running executable to the enclosing `.app` bundle.
/// A bundled launch runs `…/daruda.app/Contents/MacOS/daruda`, so the
/// first ancestor whose extension is `app` is the bundle. Returns `None`
/// for a dev / `cargo run` binary that has no `.app` ancestor.
fn app_bundle_from_exe(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn updater_with(status: AutoUpdateStatus) -> Updater {
        Updater {
            status,
            current: semver::Version::new(0, 2, 0),
            app_bundle: None,
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
            dmg_url: "https://github.com/x.dmg".to_string(),
            notes: String::new(),
        };
        for status in [
            AutoUpdateStatus::Idle,
            AutoUpdateStatus::UpToDate,
            AutoUpdateStatus::Available(info),
            AutoUpdateStatus::ReadyToRestart(PathBuf::from("/Applications/daruda.app")),
            AutoUpdateStatus::Errored("boom".to_string()),
        ] {
            assert!(
                !updater_with(status.clone()).is_busy(),
                "{status:?} must not count as busy"
            );
        }
    }

    #[test]
    fn app_bundle_from_bundled_exe_path() {
        let exe = Path::new("/Applications/daruda.app/Contents/MacOS/daruda");
        assert_eq!(
            app_bundle_from_exe(exe),
            Some(PathBuf::from("/Applications/daruda.app"))
        );
    }

    #[test]
    fn app_bundle_from_dev_exe_is_none() {
        let exe = Path::new("/Users/dev/daruda/target/debug/daruda");
        assert_eq!(app_bundle_from_exe(exe), None);
    }

    #[test]
    fn can_install_tracks_app_bundle() {
        let mut updater = updater_with(AutoUpdateStatus::Idle);
        assert!(!updater.can_install());
        updater.app_bundle = Some(PathBuf::from("/Applications/daruda.app"));
        assert!(updater.can_install());
    }
}
