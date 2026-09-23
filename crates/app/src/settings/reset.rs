//! Reset to default: the per-row affordance and the write behind it.
//!
//! Reset deletes the field's key from `config.toml` rather than writing the
//! default value, so the file keeps following the default if it changes.

use daruda_config::{Config, SettingsPatch};
use gpui::{Context, Window};

use super::search::Target;
use super::{SettingsView, spec};
use crate::surface::strings as s;

/// The field `target` edits, as the patch `Config::default()` holds.
/// `None` for a target with no single field of its own.
pub(super) fn default_patch(target: Target) -> Option<SettingsPatch> {
    let defaults = Config::default();
    match target {
        Target::Text(t) => Some((spec::text_spec(t).current)(&defaults)),
        Target::Select(v) => Some((spec::select_spec(v).current)(&defaults)),
        Target::Bool(b) => {
            let row = spec::bool_spec(b);
            Some((row.patch)((row.show)(&defaults)))
        }
        Target::StatusBarItem(_) | Target::Page(_) => None,
    }
}

impl SettingsView {
    /// Whether the saved value of `target` differs from its default — the
    /// only time its row offers Reset.
    pub(super) fn differs_from_default(&self, target: Target) -> bool {
        default_patch(target)
            .is_some_and(|patch| patch.field_changed_between(&self.base_config, &Config::default()))
    }

    /// Delete `target`'s key, then show the default in its widget.
    pub(super) fn reset_to_default(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui::BorrowAppContext as _;

        let Some(default) = default_patch(target) else {
            return;
        };
        let baseline = self.base_config.clone();
        let result = cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.reset_field_if_unchanged(default.clone(), &baseline)
        });
        match result {
            Ok(()) => {
                let live = crate::settings_store::SettingsStore::global(cx)
                    .user()
                    .clone();
                self.load_settings_patch(&default, &live, window, cx);
                self.advance_base_field_from_live(&default, cx);
                self.error = None;
                self.conflict = None;
                cx.notify();
            }
            Err(daruda_config::SettingsPatchApplyError::Conflict(_)) => {
                self.error = None;
                self.conflict = Some(default);
                cx.notify();
            }
            Err(daruda_config::SettingsPatchApplyError::Persistence(message)) => {
                self.report_save_failure(
                    default.field(),
                    &message,
                    s::settings_err_save_settings,
                    cx,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{BoolSetting, TextSetting};

    #[test]
    fn a_default_patch_names_the_field_its_row_edits() {
        let patch = default_patch(Target::Text(TextSetting::ScrollbackMaxRows)).unwrap();
        assert_eq!(patch.field().path(), "scrollback.max_rows");
        let patch = default_patch(Target::Bool(BoolSetting::WindowBlur)).unwrap();
        assert_eq!(patch.field().path(), "window.blur");
        assert!(default_patch(Target::Page(daruda_config::BuiltinSection::Font)).is_none());
    }
}
