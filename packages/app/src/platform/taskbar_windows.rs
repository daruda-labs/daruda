//! Numeric taskbar overlays with retry and Explorer-recreation recovery.

use std::cell::Cell;

mod lifecycle;

mod state;

#[derive(Clone, Copy, Default)]
struct Runtime {
    overlay: state::Overlay,
    failure_reported: bool,
}

thread_local! {
    static OVERLAY: Cell<Runtime> = const { Cell::new(Runtime {
        overlay: state::Overlay::new(), failure_reported: false,
    }) };
}

use windows_api::Win32::Foundation::HWND;
use windows_api::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows_api::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
use windows_api::Win32::UI::WindowsAndMessaging::{CreateIcon, DestroyIcon, HICON};
use windows_api::core::{HSTRING, PCWSTR};

const DIGITS: [[u8; 7]; 10] = [
    [14, 17, 19, 21, 25, 17, 14],
    [4, 12, 4, 4, 4, 4, 14],
    [14, 17, 1, 2, 4, 8, 31],
    [30, 1, 1, 14, 1, 1, 30],
    [2, 6, 10, 18, 31, 2, 2],
    [31, 16, 16, 30, 1, 1, 30],
    [14, 16, 16, 30, 17, 17, 14],
    [31, 1, 2, 4, 8, 8, 8],
    [14, 17, 17, 14, 17, 17, 14],
    [14, 17, 17, 15, 1, 1, 14],
];

pub(super) fn set(count: usize) {
    OVERLAY.with(|state| {
        let mut runtime = state.get();
        runtime.overlay.request(count);
        state.set(runtime);
    });
    refresh();
}

pub(super) fn install(cx: &mut gpui::App) {
    if cfg!(test) {
        return;
    }
    crate::watcher_pumps::spawn_periodic_pump(
        std::time::Duration::from_millis(500),
        |_| refresh(),
        cx,
    );
}

fn refresh() {
    let mut state = OVERLAY.with(Cell::get);
    let result = (|| -> anyhow::Result<()> {
        let (windows, invalidated) = lifecycle::windows()?;
        state
            .overlay
            .synchronize(invalidated, !windows.is_empty(), |count| {
                update(count, windows)
            })
    })();
    if let Err(error) = result {
        state.overlay.invalidate();
        if !state.failure_reported {
            super::report_error(
                "desktop.taskbar",
                "Taskbar overlay update failed",
                error.as_ref(),
            );
        }
        state.failure_reported = true;
    } else {
        state.failure_reported = false;
    }
    OVERLAY.with(|overlay| overlay.set(state));
}

fn update(count: usize, windows: Vec<windows_sys::Win32::Foundation::HWND>) -> anyhow::Result<()> {
    if windows.is_empty() {
        return Ok(());
    }
    // SAFETY: COM is initialized by GPUI/notification startup on this thread.
    let taskbar: ITaskbarList3 =
        unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)? };
    let icon = if count == 0 {
        HICON::default()
    } else {
        let pixels = pixels(count);
        // SAFETY: CreateIcon copies the 16x16 BGRA pixels and 1-bit mask.
        unsafe { CreateIcon(None, 16, 16, 1, 32, [0u8; 32].as_ptr(), pixels.as_ptr())? }
    };
    let description = HSTRING::from(count.to_string());
    let result = (|| -> windows_api::core::Result<()> {
        // SAFETY: all HWNDs belong to this process; SetOverlayIcon copies its icon.
        unsafe {
            taskbar.HrInit()?;
            for window in windows {
                taskbar.SetOverlayIcon(HWND(window), icon, PCWSTR(description.as_ptr()))?;
            }
        }
        Ok(())
    })();
    if count != 0 {
        // SAFETY: this function created and exclusively owns the temporary icon.
        unsafe {
            DestroyIcon(icon)?;
        }
    }
    result.map_err(Into::into)
}

fn pixels(count: usize) -> [u8; 16 * 16 * 4] {
    let mut bytes = [0; 16 * 16 * 4];
    for y in 0..16_i32 {
        for x in 0..16_i32 {
            if (x * 2 - 15).pow(2) + (y * 2 - 15).pow(2) <= 225 {
                let offset = ((15 - y) * 16 + x) as usize * 4;
                bytes[offset..offset + 4].copy_from_slice(&[50, 50, 210, 255]);
            }
        }
    }
    let text = count.min(99).to_string();
    let start = if text.len() == 1 { 5 } else { 2 };
    for (index, digit) in text.bytes().enumerate() {
        for (y, row) in DIGITS[(digit - b'0') as usize].iter().enumerate() {
            for x in 0..5 {
                if row & (1 << (4 - x)) != 0 {
                    let offset = ((15 - (y + 4)) * 16 + start + index * 6 + x) * 4;
                    bytes[offset..offset + 4].copy_from_slice(&[255; 4]);
                }
            }
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    #[test]
    fn overlay_has_transparent_corners_and_distinct_counts() {
        let one = super::pixels(1);
        assert_eq!(&one[..4], &[0; 4]);
        assert_ne!(one, super::pixels(2));
        assert_eq!(super::pixels(99), super::pixels(100));
    }
}
