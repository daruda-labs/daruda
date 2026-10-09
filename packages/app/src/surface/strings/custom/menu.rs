use crate::surface::constants::APP_NAME;

pub(crate) fn app() -> &'static str {
    APP_NAME
}

/// Only the Windows desktop integration exposes a tray menu.
#[cfg(windows)]
pub(crate) fn tray_show() -> String {
    rust_i18n::t!("menu.tray_show").into_owned()
}

/// Only the Windows tray exposes this update action.
#[cfg(windows)]
pub(crate) fn tray_check_updates() -> String {
    rust_i18n::t!("menu.tray_check_updates").into_owned()
}
