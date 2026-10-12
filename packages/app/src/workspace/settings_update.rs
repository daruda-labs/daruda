//! The app's self-updater as Settings' About page sees it: its state pushed
//! in as a [`UpdateState`], and the page's buttons answered.

use gpui::App;

use crate::update::{AutoUpdateStatus, InstallGate, Updater};
use daruda_settings::{UpdateCommand, UpdateInstall, UpdateState};

/// The updater's current step; [`UpdateState::Idle`] when this build has none.
pub(in crate::workspace) fn update_state(cx: &mut App) -> UpdateState {
    let Some(updater) = Updater::get(cx) else {
        return UpdateState::Idle;
    };
    let updater = updater.read(cx);
    state_of(updater.status(), updater.install_gate())
}

fn state_of(status: &AutoUpdateStatus, gate: &InstallGate) -> UpdateState {
    match status {
        AutoUpdateStatus::Idle => UpdateState::Idle,
        AutoUpdateStatus::Checking => UpdateState::Checking,
        AutoUpdateStatus::UpToDate => UpdateState::UpToDate,
        AutoUpdateStatus::Errored(message) => UpdateState::Errored(message.clone()),
        AutoUpdateStatus::Available(info) => UpdateState::Available {
            version: info.version.to_string(),
            notes: info.notes.clone(),
            install: match gate {
                InstallGate::Target(_) => UpdateInstall::InPlace,
                InstallGate::ManualPackage => UpdateInstall::ManualPackage,
                InstallGate::Unpackaged => UpdateInstall::Unpackaged,
            },
        },
        AutoUpdateStatus::Downloading => UpdateState::Downloading,
        AutoUpdateStatus::Installing => UpdateState::Installing,
        AutoUpdateStatus::ReadyToRestart(target) => UpdateState::ReadyToRestart {
            uses_installer: target.uses_installer(),
        },
    }
}

/// Run an About page button against the updater, if this build has one.
pub(in crate::workspace) fn run(command: UpdateCommand, cx: &mut App) {
    let Some(updater) = Updater::get(cx) else {
        return;
    };
    updater.update(cx, |updater, cx| match command {
        UpdateCommand::Check => updater.check(cx),
        UpdateCommand::DownloadAndInstall => updater.download_and_install(cx),
        UpdateCommand::Restart => updater.restart(cx),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_is_offered_only_where_it_can_be_installed() {
        for (gate, install) in [
            (InstallGate::ManualPackage, UpdateInstall::ManualPackage),
            (InstallGate::Unpackaged, UpdateInstall::Unpackaged),
        ] {
            let info = daruda_update::ReleaseInfo {
                version: semver::Version::new(9, 9, 9),
                tag: "v9.9.9".to_string(),
                asset_url: String::new(),
                asset_name: String::new(),
                checksums_url: String::new(),
                notes: "notes".to_string(),
            };
            assert_eq!(
                state_of(&AutoUpdateStatus::Available(info), &gate),
                UpdateState::Available {
                    version: "9.9.9".to_string(),
                    notes: "notes".to_string(),
                    install,
                }
            );
        }
    }

    #[test]
    fn a_failed_step_carries_its_reason() {
        assert_eq!(
            state_of(
                &AutoUpdateStatus::Errored("offline".to_string()),
                &InstallGate::Unpackaged
            ),
            UpdateState::Errored("offline".to_string())
        );
    }
}
