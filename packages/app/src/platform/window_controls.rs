//! Size native caption buttons without replacing their AppKit behavior.

#[cfg(not(target_os = "macos"))]
pub(crate) fn compact(_window: &gpui::Window) {}

#[cfg(target_os = "macos")]
pub(crate) fn compact(window: &gpui::Window) {
    use objc2_app_kit::{NSView, NSWindowButton};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // Fullscreen's revealed title bar belongs entirely to AppKit.
    if window.is_fullscreen() {
        return;
    }
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: GPUI lends its live AppKit content view for `window`'s lifetime.
    // Window is main-thread-only; the view is neither retained nor stored here.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(native_window) = view.window() else {
        return;
    };
    for (index, kind) in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ]
    .into_iter()
    .enumerate()
    {
        if let Some(button) = native_window.standardWindowButton(kind) {
            compact_button(&button, index);
        }
    }
}

#[cfg(target_os = "macos")]
fn compact_button(button: &objc2_app_kit::NSButton, index: usize) {
    let Some(cell) = button.cell() else {
        return;
    };
    button.setFrame(compact_frame(button.frame(), index));
    // Preserve the native drawing coordinates so the artwork scales with
    // its frame instead of clipping the standard-sized cell and hover glyphs.
    button.setBoundsSize(cell.cellSize());
    objc2_app_kit::NSView::setNeedsDisplay(button, true);
}

#[cfg(target_os = "macos")]
fn compact_frame(mut frame: objc2_foundation::NSRect, index: usize) -> objc2_foundation::NSRect {
    use crate::ui::theme;

    let diameter = f64::from(theme::TRAFFIC_LIGHT_SIZE);
    // Recompute the whole cluster instead of inheriting AppKit's wider pitch.
    frame.origin.x = f64::from(theme::TRAFFIC_LIGHT_X)
        + index as f64 * (diameter + f64::from(theme::TRAFFIC_LIGHT_GAP));
    // Keep GPUI's top inset: AppKit frame coordinates grow upward.
    frame.origin.y += frame.size.height - diameter;
    frame.size = objc2_foundation::NSSize::new(diameter, diameter);
    frame
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::compact_frame;
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    #[test]
    fn resizing_preserves_top_inset_without_drifting_on_reapply() {
        for (index, x) in [8.0, 31.0, 54.0].into_iter().enumerate() {
            let original = NSRect::new(NSPoint::new(x, 10.0), NSSize::new(14.0, 14.0));
            let compact = compact_frame(original, index);
            assert_eq!(
                compact,
                NSRect::new(
                    NSPoint::new([8.0, 28.0, 48.0][index], 12.0),
                    NSSize::new(12.0, 12.0)
                )
            );
            assert_eq!(compact_frame(compact, index), compact);
        }
    }

    #[test]
    fn native_relayout_cannot_leave_uneven_gaps() {
        let frames =
            [9.0, 32.0, 55.0].map(|x| NSRect::new(NSPoint::new(x, 9.0), NSSize::new(12.0, 12.0)));
        let compact: Vec<_> = frames
            .into_iter()
            .enumerate()
            .map(|(index, frame)| compact_frame(frame, index))
            .collect();
        for pair in compact.windows(2) {
            assert_eq!(
                pair[1].origin.x - (pair[0].origin.x + pair[0].size.width),
                8.0
            );
        }
        assert_eq!(compact[2].origin.x + compact[2].size.width, 60.0);
    }
}
