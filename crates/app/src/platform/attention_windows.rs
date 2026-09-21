//! Win32 foreground, session idle time, and taskbar attention queries.

use daruda_terminal::AttentionKind;
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FLASHW_STOP, FLASHW_TIMERNOFG, FLASHW_TRAY, FLASHWINFO, FlashWindowEx,
    GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible,
};
use windows_sys::core::BOOL;

pub fn is_app_active() -> bool {
    let mut pid = 0;
    // SAFETY: the window handle is borrowed from Win32 and pid is writable.
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut pid) };
    pid == std::process::id()
}

pub fn system_idle_seconds() -> Option<f64> {
    let mut input = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: input has the required size and remains valid for the query.
    if unsafe { GetLastInputInfo(&mut input) } == 0 {
        return None;
    }
    // SAFETY: GetTickCount has no pointer arguments or ownership transfer.
    Some(elapsed_seconds(unsafe { GetTickCount() }, input.dwTime))
}

fn elapsed_seconds(now: u32, last: u32) -> f64 {
    f64::from(now.wrapping_sub(last)) / 1000.0
}

pub fn apply(kind: AttentionKind) {
    let flags = match kind {
        AttentionKind::Cancel => FLASHW_STOP,
        AttentionKind::Once => FLASHW_TRAY,
        AttentionKind::Critical => FLASHW_TRAY | FLASHW_TIMERNOFG,
    };
    // SAFETY: EnumWindows invokes the callback synchronously; flags is a value.
    unsafe { EnumWindows(Some(flash_owned_window), flags as LPARAM) };
}

unsafe extern "system" fn flash_owned_window(window: HWND, flags: LPARAM) -> BOOL {
    let mut pid = 0;
    // SAFETY: EnumWindows supplied the handle and pid is valid for the call.
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    // SAFETY: the callback's handle remains borrowed throughout this call.
    if pid == std::process::id() && unsafe { IsWindowVisible(window) } != 0 {
        let flash = FLASHWINFO {
            cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
            hwnd: window,
            dwFlags: flags as u32,
            uCount: 1,
            dwTimeout: 0,
        };
        // SAFETY: flash contains a valid borrowed handle and correct size.
        unsafe { FlashWindowEx(&flash) };
    }
    1
}

#[cfg(test)]
mod tests {
    use super::elapsed_seconds;

    #[test]
    fn idle_ticks_wrap_after_49_days() {
        assert_eq!(elapsed_seconds(500, u32::MAX - 499), 1.0);
        assert_eq!(elapsed_seconds(1500, 0), 1.5);
    }
}
