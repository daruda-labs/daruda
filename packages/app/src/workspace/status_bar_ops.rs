//! Status bar's right-click toggle menu — the mutation side.
//! `status_bar/context_menu.rs` builds the menu and dispatches here;
//! this file owns the `StatusBarConfig` persistence call chain.

use daruda_config::{KeepAwakeMode, StatusBarItem};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{BorrowAppContext as _, Context};

use crate::workspace::Workspace;
use crate::workspace::status_bar::KeepAwakeView;

impl Workspace {
    /// Flip `item`'s membership in `StatusBarConfig::hidden_items` and
    /// persist it through `SettingsStore::apply_patch`, so the choice survives
    /// restart. The
    /// Global's `observe_global` fan-out re-applies the resolved config
    /// to every open workspace (including this one), refreshing
    /// `self.mirrors.status_bar` without a separate `cx.notify()` here.
    ///
    /// Do not call this from a test: `apply_patch` writes the real
    /// on-disk `config_path()` with no test-mode redirect (see
    /// `settings/tests.rs::validate_does_not_revert_background_pairing`).
    /// `StatusBarConfig::toggle` carries the actual membership-flip logic
    /// and is unit-tested there instead.
    pub(in crate::workspace) fn toggle_status_bar_item(
        &mut self,
        item: StatusBarItem,
        cx: &mut Context<Self>,
    ) {
        let result = cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.apply_patch(daruda_config::SettingsPatch::ToggleStatusBarItem(item))
        });
        if let Err(msg) = result {
            self.report_error(
                ErrorReport::new(crate::surface::strings::error::status_bar_save_failed())
                    .severity(ErrorSeverity::Warning)
                    .message(msg)
                    .at(file!(), line!())
                    .dedup("status_bar.toggle_item")
                    .build(),
                cx,
            );
        }
    }

    /// The keep-awake segment's snapshot; the status bar decides whether
    /// to draw it, like every other segment.
    pub(in crate::workspace) fn keep_awake_view(&self, cx: &gpui::App) -> KeepAwakeView {
        KeepAwakeView {
            mode: crate::keep_awake::mode(cx),
            status: crate::keep_awake::status(cx),
        }
    }

    /// Persist the keep-awake mode the status-bar menu picked. The policy
    /// observes the settings store, so it re-decides on its own.
    pub(in crate::workspace) fn set_keep_awake_mode(
        &mut self,
        mode: KeepAwakeMode,
        cx: &mut Context<Self>,
    ) {
        let result = cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.apply_patch(daruda_config::SettingsPatch::KeepAwake(mode))
        });
        if let Err(msg) = result {
            self.report_error(
                ErrorReport::new(crate::surface::strings::error::keep_awake_save_failed())
                    .severity(ErrorSeverity::Warning)
                    .message(msg)
                    .at(file!(), line!())
                    .dedup("status_bar.keep_awake_mode")
                    .build(),
                cx,
            );
        }
    }
}
