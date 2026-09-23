//! Material Symbols used by application controls, separate from file icons.

use gpui::px;
use gpui_component::{Icon, Sizable as _};

use super::theme;

pub const SETTINGS: &str = "icons/ui/settings.svg";
pub const TERMINAL: &str = "icons/ui/terminal.svg";
pub const TEXT_FIELDS: &str = "icons/ui/text-fields.svg";
pub const CURSOR: &str = "icons/ui/cursor.svg";
pub const KEYBOARD: &str = "icons/ui/keyboard.svg";
pub const DNS: &str = "icons/ui/dns.svg";
pub const PERSON: &str = "icons/ui/person.svg";
pub const NOTIFICATIONS: &str = "icons/ui/notifications.svg";
pub const EXTENSION: &str = "icons/ui/extension.svg";
pub const BUILD: &str = "icons/ui/build.svg";
pub const INFO: &str = "icons/ui/info.svg";
pub const AGENT: &str = "icons/bot.svg";
pub const CODE: &str = "icons/ui/code.svg";
pub const DOCK: &str = "icons/ui/dock-to-left.svg";
pub const CLOSE: &str = "icons/ui/close.svg";
pub const DELETE: &str = "icons/ui/delete.svg";
pub const EDIT: &str = "icons/ui/edit.svg";
pub const UNDO: &str = "icons/ui/undo.svg";
pub const ADD: &str = "icons/ui/add.svg";
pub const MINIMIZE: &str = "icons/ui/minimize.svg";
pub const MAXIMIZE: &str = "icons/ui/crop-square.svg";
pub const RESTORE: &str = "icons/ui/filter-none.svg";
pub const PREVIOUS: &str = "icons/ui/keyboard-arrow-up.svg";
pub const NEXT: &str = "icons/ui/keyboard-arrow-down.svg";
pub const EXPAND_MORE: &str = "icons/ui/expand-more.svg";
pub const CHEVRON_RIGHT: &str = "icons/ui/chevron-right.svg";
pub const BACK: &str = "icons/ui/arrow-back.svg";
pub const FORWARD: &str = "icons/ui/arrow-forward.svg";
pub const PIN: &str = "icons/ui/keep.svg";
pub const PIN_FILLED: &str = "icons/ui/keep-fill.svg";
pub const RECORD: &str = "icons/ui/fiber-manual-record.svg";
pub const CHECKBOX_OFF: &str = "icons/ui/check-box-outline-blank.svg";
pub const CHECKBOX_ON: &str = "icons/ui/check-box.svg";
pub const CHECKBOX_MIXED: &str = "icons/ui/indeterminate-check-box.svg";
pub const RADIO_OFF: &str = "icons/ui/radio-button-unchecked.svg";
pub const RADIO_ON: &str = "icons/ui/radio-button-checked.svg";
pub const REFRESH: &str = "icons/ui/refresh.svg";
pub const COPY: &str = "icons/ui/content-copy.svg";
pub const CHECK: &str = "icons/ui/check.svg";
pub const EXPAND: &str = "icons/ui/open-in-full.svg";
pub const HISTORY: &str = "icons/ui/history.svg";
pub const VISIBILITY: &str = "icons/ui/visibility.svg";
pub const DIFFERENCE: &str = "icons/ui/difference.svg";
// Lucide (ISC) outlines, drawn at the dock's lighter 1.65 stroke.
pub const TASKS: &str = "icons/lucide/list-checks.svg";
pub const FLOWS: &str = "icons/lucide/workflow.svg";
pub const FOLDER: &str = "icons/lucide/folder.svg";
pub const SESSION: &str = "icons/lucide/message-square.svg";
pub const SKILL: &str = "icons/lucide/file-text.svg";
pub const SERVER: &str = "icons/lucide/server.svg";

/// Explicit pixels keep controls independent of font and button size tiers.
pub fn icon(path: &'static str) -> Icon {
    Icon::empty()
        .path(path)
        .with_size(px(theme::CONTROL_ICON_SIZE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AssetSource as _;

    #[test]
    fn every_control_symbol_is_embedded() {
        for path in [
            BUILD,
            SETTINGS,
            CLOSE,
            DELETE,
            EDIT,
            UNDO,
            ADD,
            MINIMIZE,
            MAXIMIZE,
            RESTORE,
            PREVIOUS,
            NEXT,
            EXPAND_MORE,
            CHEVRON_RIGHT,
            BACK,
            FORWARD,
            PIN,
            PIN_FILLED,
            RECORD,
            CHECKBOX_OFF,
            CHECKBOX_ON,
            CHECKBOX_MIXED,
            RADIO_OFF,
            RADIO_ON,
            REFRESH,
            COPY,
            CHECK,
            EXPAND,
            HISTORY,
            VISIBILITY,
            DIFFERENCE,
            TASKS,
            FLOWS,
            FOLDER,
            SESSION,
            SKILL,
            SERVER,
        ] {
            let bytes = crate::assets::DarudaAssets.load(path).unwrap().unwrap();
            assert!(bytes.windows(4).any(|w| w == b"<svg"), "{path}");
        }
    }
}
