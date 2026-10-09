//! Capture the visible client surface, using Windows' native pixel and DPI stack.

use anyhow::{Context as _, Result};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::*,
    Storage::Xps::{PW_CLIENTONLY, PrintWindow},
    UI::WindowsAndMessaging::{GetClientRect, IsWindowVisible, PW_RENDERFULLCONTENT},
};

struct Surface {
    window: HWND,
    source: HDC,
    target: HDC,
    bitmap: HBITMAP,
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: each resource is owned here and closed exactly once.
        unsafe {
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            if !self.target.is_null() {
                DeleteDC(self.target);
            }
            if !self.source.is_null() {
                ReleaseDC(self.window, self.source);
            }
        }
    }
}

pub(crate) fn capture(window: &gpui::Window) -> Result<image::RgbaImage> {
    let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("Read native window handle: {error}"))?
        .as_raw()
    else {
        anyhow::bail!("Window has no Win32 handle");
    };
    let hwnd = handle.hwnd.get() as HWND;
    // A hidden-consoles launcher can also hide a GUI's first window.
    // Reject that state rather than declaring a blank image a successful capture.
    anyhow::ensure!(
        unsafe { IsWindowVisible(hwnd) } != 0,
        "Windows capture requires a visible window"
    );
    let mut bounds = RECT::default();
    // SAFETY: GPUI owns a live HWND throughout this synchronous capture.
    if unsafe { GetClientRect(hwnd, &mut bounds) } == 0 {
        return Err(std::io::Error::last_os_error()).context("Read client bounds");
    }
    let width = bounds.right - bounds.left;
    let height = bounds.bottom - bounds.top;
    anyhow::ensure!(
        width > 0 && height > 0 && i64::from(width) * i64::from(height) <= 16_777_216,
        "Invalid capture dimensions"
    );
    let mut surface = Surface {
        window: hwnd,
        source: std::ptr::null_mut(),
        target: std::ptr::null_mut(),
        bitmap: std::ptr::null_mut(),
    };
    // SAFETY: a compatible bitmap is selected only during PrintWindow, then removed
    // before GetDIBits and Drop. All handles remain owned by Surface.
    unsafe {
        surface.source = GetDC(hwnd);
        anyhow::ensure!(!surface.source.is_null(), "Cannot read window pixels");
        surface.target = CreateCompatibleDC(surface.source);
        anyhow::ensure!(!surface.target.is_null(), "Cannot allocate capture context");
        surface.bitmap = CreateCompatibleBitmap(surface.source, width, height);
        anyhow::ensure!(!surface.bitmap.is_null(), "Cannot allocate capture bitmap");
        let previous = SelectObject(surface.target, surface.bitmap);
        anyhow::ensure!(
            !previous.is_null() && previous as isize != GDI_ERROR as isize,
            "Cannot select capture bitmap"
        );
        // DirectComposition pixels are not present in the HWND's GDI DC.
        // Ask DWM for the composed surface rather than copying a black DC.
        let copied = PrintWindow(hwnd, surface.target, PW_CLIENTONLY | PW_RENDERFULLCONTENT);
        SelectObject(surface.target, previous);
        if copied == 0 {
            return Err(std::io::Error::last_os_error()).context("Capture window pixels");
        }
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bytes = vec![0; width as usize * height as usize * 4];
        anyhow::ensure!(
            GetDIBits(
                surface.target,
                surface.bitmap,
                0,
                height as u32,
                bytes.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS
            ) == height,
            "Incomplete capture bitmap"
        );
        for pixel in bytes.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        anyhow::ensure!(
            bytes
                .as_chunks::<4>()
                .0
                .windows(2)
                .any(|pixels| pixels[0] != pixels[1]),
            "Capture contained no rendered content"
        );
        image::RgbaImage::from_raw(width as u32, height as u32, bytes)
            .context("Invalid capture pixel layout")
    }
}
