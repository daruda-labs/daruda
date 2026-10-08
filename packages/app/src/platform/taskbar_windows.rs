//! Numeric taskbar overlays with retry and Explorer-recreation recovery.

use std::cell::Cell;

mod lifecycle;

#[derive(Clone, Copy, Default)]
struct Overlay {
    desired: usize,
    applied: Option<usize>,
    failure_reported: bool,
}

impl Overlay {
    fn synchronize(
        &mut self,
        invalidated: bool,
        available: bool,
        apply: impl FnOnce(usize) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        if invalidated {
            self.applied = None;
        }
        if available && self.applied != Some(self.desired) {
            self.applied = None;
            apply(self.desired)?;
            self.applied = Some(self.desired);
        }
        Ok(())
    }
}

thread_local! {
    static OVERLAY: Cell<Overlay> = const { Cell::new(Overlay {
        desired: 0, applied: None, failure_reported: false,
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
        state.set(Overlay {
            desired: count,
            ..state.get()
        })
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
        state.synchronize(invalidated, !windows.is_empty(), |count| {
            update(count, windows)
        })
    })();
    if let Err(error) = result {
        state.applied = None;
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
    fn unchanged_count_retries_after_failure_and_taskbar_recreation() {
        let mut state = super::Overlay {
            desired: 3,
            ..Default::default()
        };
        assert!(
            state
                .synchronize(false, true, |_| anyhow::bail!("temporary failure"))
                .is_err()
        );
        assert_eq!(state.applied, None);
        let mut attempts = 0;
        for invalidated in [false, false, true] {
            state
                .synchronize(invalidated, true, |count| {
                    assert_eq!(count, 3);
                    attempts += 1;
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(attempts, 2);
        state.desired = 0;
        state
            .synchronize(false, false, |_| panic!("no window"))
            .unwrap();
        assert_eq!(state.applied, Some(3));
        state
            .synchronize(false, true, |count| {
                assert_eq!(count, 0);
                Ok(())
            })
            .unwrap();
        assert_eq!(state.applied, Some(0));
    }

    #[test]
    fn overlay_has_transparent_corners_and_distinct_counts() {
        let one = super::pixels(1);
        assert_eq!(&one[..4], &[0; 4]);
        assert_ne!(one, super::pixels(2));
        assert_eq!(super::pixels(99), super::pixels(100));
    }
}
