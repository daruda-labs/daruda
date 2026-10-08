//! Main-thread window subscriptions; callbacks only invalidate cached overlays.

use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Shell::{
    DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, RegisterWindowMessageW, WM_NCDESTROY,
};

static CREATED: AtomicU32 = AtomicU32::new(0);
thread_local! {
    static INVALIDATED: Cell<bool> = const { Cell::new(false) };
}
const SUBCLASS_ID: usize = 1;

pub(super) fn windows() -> anyhow::Result<(Vec<HWND>, bool)> {
    if CREATED.load(Ordering::Relaxed) == 0 {
        let name: Vec<_> = "TaskbarButtonCreated"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: the name is a live, terminated UTF-16 buffer.
        let message = unsafe { RegisterWindowMessageW(name.as_ptr()) };
        anyhow::ensure!(message != 0, "could not register taskbar creation message");
        CREATED.store(message, Ordering::Relaxed);
    }
    let mut windows = Vec::<HWND>::new();
    // SAFETY: synchronous enumeration borrows the Vec for the callback duration.
    if unsafe { EnumWindows(Some(collect), (&mut windows as *mut Vec<HWND>) as LPARAM) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut added = false;
    for &window in &windows {
        let mut data = 0;
        // SAFETY: enumeration selected this process's live windows. This pump
        // runs on GPUI's GUI thread, the owner of these window procedures.
        unsafe {
            if GetWindowSubclass(window, Some(procedure), SUBCLASS_ID, &mut data) == 0 {
                anyhow::ensure!(
                    SetWindowSubclass(window, Some(procedure), SUBCLASS_ID, 0) != 0,
                    "could not subscribe to taskbar recreation"
                );
                added = true;
            }
        }
    }
    Ok((windows, INVALIDATED.replace(false) || added))
}

unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    if message == CREATED.load(Ordering::Relaxed) || message == WM_NCDESTROY {
        INVALIDATED.set(true);
    }
    // SAFETY: Windows supplies this borrowed HWND and message. Unsubscribe
    // before destruction; all other behavior remains with GPUI's procedure.
    unsafe {
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(window, Some(procedure), id);
        }
        DefSubclassProc(window, message, wparam, lparam)
    }
}

unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> windows_sys::core::BOOL {
    let mut pid = 0;
    // SAFETY: EnumWindows supplies a borrowed HWND and our live Vec pointer.
    unsafe {
        let thread = GetWindowThreadProcessId(window, &mut pid);
        if pid == std::process::id()
            && thread == GetCurrentThreadId()
            && IsWindowVisible(window) != 0
        {
            (*(data as *mut Vec<HWND>)).push(window);
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SendMessageW,
    };

    #[test]
    fn native_recreation_message_invalidates_without_replacing_window_behavior() {
        windows().unwrap();
        let class: Vec<_> = "STATIC".encode_utf16().chain(Some(0)).collect();
        // SAFETY: creates a hidden, test-owned window on this test thread.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!window.is_null());
        struct OwnedWindow(HWND);
        impl Drop for OwnedWindow {
            fn drop(&mut self) {
                // SAFETY: the test exclusively owns this same-thread window.
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        let window = OwnedWindow(window);
        // SAFETY: callback has static lifetime; message delivery is synchronous.
        unsafe {
            assert_ne!(
                SetWindowSubclass(window.0, Some(procedure), SUBCLASS_ID, 0),
                0
            );
            INVALIDATED.set(false);
            SendMessageW(window.0, CREATED.load(Ordering::Relaxed), 0, 0);
            assert!(INVALIDATED.replace(false));
            let mut data = 0;
            assert_ne!(
                GetWindowSubclass(window.0, Some(procedure), SUBCLASS_ID, &mut data),
                0
            );
        }
        drop(window);
        assert!(INVALIDATED.replace(false));
    }
}
