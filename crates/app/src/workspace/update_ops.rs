//! Workspace-side reaction to `Updater` status transitions.
//!
//! The `Updater` entity self-notifies on every status change, so the
//! subscription installed in `new_with_project_impl` fires on each one.
//! This handler filters for the single case the user cares about —
//! a newer version being `Available` — and surfaces it as an
//! informational toast, deduped so repeated notifies for the same
//! version don't spam.

use crate::workspace::Workspace;

impl Workspace {
    /// Toast when the updater reports a newer version is available.
    /// Every other status transition is ignored here (the About page
    /// owns the full lifecycle UI).
    pub(in crate::workspace) fn on_updater_status_changed(
        &mut self,
        updater: &gpui::Entity<crate::update::Updater>,
        cx: &mut gpui::Context<Self>,
    ) {
        use crate::update::AutoUpdateStatus;
        let AutoUpdateStatus::Available(info) = updater.read(cx).status() else {
            return;
        };
        let version = info.version.clone();
        if !should_announce_update(&mut self.last_update_toast_version, &version) {
            return; // already toasted this version
        }
        let report = daruda_store::observability::error_report::ErrorReport::new(
            crate::surface::strings::update_available_toast(&version.to_string()),
        )
        .severity(daruda_store::observability::error_report::ErrorSeverity::Info)
        .dedup(format!("update.available.{version}"))
        .build();
        self.report_error(report, cx);
    }
}

/// Returns true the first time a given version should be announced,
/// updating the remembered version. Repeat calls for the same version
/// return false (dedup); a new version returns true again.
fn should_announce_update(last: &mut Option<semver::Version>, candidate: &semver::Version) -> bool {
    if last.as_ref() == Some(candidate) {
        return false;
    }
    *last = Some(candidate.clone());
    true
}

#[cfg(test)]
mod tests {
    use super::should_announce_update;
    use semver::Version;

    #[test]
    fn fresh_version_announces() {
        let mut last = None;
        assert!(should_announce_update(&mut last, &Version::new(0, 3, 0)));
        assert_eq!(last, Some(Version::new(0, 3, 0)));
    }

    #[test]
    fn same_version_repeated_is_deduped() {
        let mut last = None;
        let v = Version::new(0, 3, 0);
        assert!(should_announce_update(&mut last, &v));
        assert!(!should_announce_update(&mut last, &v));
        assert!(!should_announce_update(&mut last, &v));
    }

    #[test]
    fn new_version_announces_again() {
        let mut last = None;
        assert!(should_announce_update(&mut last, &Version::new(0, 3, 0)));
        assert!(should_announce_update(&mut last, &Version::new(0, 4, 0)));
        assert_eq!(last, Some(Version::new(0, 4, 0)));
    }

    #[test]
    fn returning_to_prior_version_announces() {
        // State only remembers the last announced version, so going back
        // to an older version is treated as a fresh announcement.
        let mut last = None;
        assert!(should_announce_update(&mut last, &Version::new(0, 4, 0)));
        assert!(should_announce_update(&mut last, &Version::new(0, 3, 0)));
        assert!(should_announce_update(&mut last, &Version::new(0, 4, 0)));
    }
}
