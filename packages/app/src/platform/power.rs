//! Windows power leases and resume subscriptions, owned by the GUI thread.

#[cfg(windows)]
#[path = "power_windows.rs"]
mod windows;

pub(crate) fn install(cx: &mut gpui::App) {
    #[cfg(windows)]
    windows::install(cx);
    #[cfg(not(windows))]
    let _ = cx;
}
