//! Native tray ownership and hidden-window restoration on the GUI thread.

use gpui::{App, Global};
use raw_window_handle::RawWindowHandle;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuId, MenuItem},
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IsIconic, SW_HIDE, SW_RESTORE, SW_SHOWNA, ShowWindow,
};

struct Tray {
    _icon: TrayIcon,
    show: MenuId,
    settings: MenuId,
    update: MenuId,
    quit: MenuId,
}
impl Global for Tray {}

pub(super) fn install(cx: &mut App) {
    super::super::power::install(cx);
    super::super::taskbar_windows::install(cx);
    if !crate::settings_store::SettingsStore::global(cx)
        .user()
        .desktop
        .tray_enabled
    {
        return;
    }
    match build() {
        Ok(tray) => cx.set_global(tray),
        Err(error) => {
            super::super::report_error(
                "desktop.tray",
                "Windows tray initialization failed",
                error.as_ref(),
            );
            return;
        }
    }
    crate::watcher_pumps::spawn_periodic_pump(
        std::time::Duration::from_millis(100),
        |cx| {
            for event in TrayIconEvent::receiver().try_iter() {
                if matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                ) {
                    reveal(cx);
                }
            }
            for event in MenuEvent::receiver().try_iter() {
                let tray = cx.global::<Tray>();
                if event.id == tray.show {
                    reveal(cx);
                } else if event.id == tray.settings {
                    reveal(cx);
                    cx.dispatch_action(&crate::workspace::OpenSettings(
                        daruda_config::BuiltinSection::default(),
                    ));
                } else if event.id == tray.update {
                    if let Some(updater) = crate::update::Updater::get(cx) {
                        updater.update(cx, |updater, cx| updater.check(cx));
                    }
                } else if event.id == tray.quit {
                    crate::workspace::Workspace::request_quit(cx);
                }
            }
        },
        cx,
    );
}

fn build() -> anyhow::Result<Tray> {
    use crate::surface::strings::menu as s;
    let menu = Menu::new();
    let show = MenuItem::new(s::tray_show(), true, None);
    let settings = MenuItem::new(s::settings(), true, None);
    let update = MenuItem::new(s::tray_check_updates(), true, None);
    let quit = MenuItem::new(s::quit_app(), true, None);
    menu.append_items(&[&show, &settings, &update, &quit])?;
    let image = image::load_from_memory(include_bytes!("../../../../assets/icon.png"))?
        .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    let icon = Icon::from_rgba(image.into_raw(), 32, 32)?;
    let tooltip = format!(
        "daruda ({})",
        daruda_store::observability::log_writer::log_profile()
    );
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(tooltip)
        .with_icon(icon)
        .build()?;
    Ok(Tray {
        _icon: tray,
        show: show.id().clone(),
        settings: settings.id().clone(),
        update: update.id().clone(),
        quit: quit.id().clone(),
    })
}

fn reveal(cx: &mut App) {
    super::reveal(None, cx);
}

fn native(window: &gpui::Window) -> Option<windows_sys::Win32::Foundation::HWND> {
    match raw_window_handle::HasWindowHandle::window_handle(window)
        .ok()?
        .as_raw()
    {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as _),
        _ => None,
    }
}

pub(super) fn show(window: &gpui::Window) {
    if let Some(hwnd) = native(window) {
        // SAFETY: GPUI lends the live HWND for this main-thread call.
        unsafe {
            ShowWindow(
                hwnd,
                if IsIconic(hwnd) != 0 {
                    SW_RESTORE
                } else {
                    SW_SHOWNA
                },
            );
        }
    }
}

pub(super) fn hide_to_tray(window: &gpui::Window, cx: &App) -> bool {
    if !cx.has_global::<Tray>()
        || !crate::settings_store::SettingsStore::global(cx)
            .user()
            .desktop
            .close_to_tray
    {
        return false;
    }
    let Some(hwnd) = native(window) else {
        return false;
    };
    // SAFETY: the tray retains a way back to this live window.
    unsafe {
        ShowWindow(hwnd, SW_HIDE);
    }
    true
}
