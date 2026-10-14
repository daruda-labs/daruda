//! `[usage] stop_on_overage` — which accounts offer the auto-stop and the
//! persistence call chain the Usage tab and the status-bar chip dispatch to.

use daruda_store::accounts::AccountRecipeId;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{BorrowAppContext as _, Context};

use crate::workspace::Workspace;

/// Whether `recipe`'s surfaces offer the auto-stop: only Claude's rate-limit
/// metadata flags a reply as billed extra usage.
pub(in crate::workspace) fn offers_overage_stop(recipe: AccountRecipeId) -> bool {
    recipe == AccountRecipeId::Claude
}

impl Workspace {
    /// Persist `[usage] stop_on_overage` through `SettingsStore::apply_patch`;
    /// the global's `observe_global` fan-out refreshes every window, so no
    /// extra notify here.
    ///
    /// Do not call this from a test: `apply_patch` writes the real on-disk
    /// config with no test-mode redirect.
    pub(in crate::workspace) fn set_stop_on_overage(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let result = cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.apply_patch(daruda_config::SettingsPatch::UsageStopOnOverage(enabled))
        });
        if let Err(msg) = result {
            self.report_error(
                ErrorReport::new(crate::surface::strings::error::usage_save_failed())
                    .severity(ErrorSeverity::Warning)
                    .message(msg)
                    .at(file!(), line!())
                    .dedup("usage.stop_on_overage")
                    .build(),
                cx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_claude_offers_the_overage_stop() {
        assert!(offers_overage_stop(AccountRecipeId::Claude));
        assert!(!offers_overage_stop(AccountRecipeId::Codex));
    }
}
