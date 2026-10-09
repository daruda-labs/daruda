//! macos attention, foreground state and idle queries.

use daruda_terminal::AttentionKind;

/// Show `count` on the Dock icon; `0` clears the badge. A no-op off the main
/// thread, which `NSApplication` requires.
pub fn set_badge_count(count: usize) {
    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        return;
    };
    let label = (count > 0).then(|| objc2_foundation::NSString::from_str(&count.to_string()));
    objc2_app_kit::NSApplication::sharedApplication(mtm)
        .dockTile()
        .setBadgeLabel(label.as_deref());
}

/// True when the daruda window is currently the focused app.
/// Used by notification gating: the "skip the focused pane" rule
/// only applies when daruda itself is foreground — if the user is
/// in another app, every pane's notification is welcome regardless
/// of which one daruda thinks is focused.
///
/// Returns `false` if called off the main thread; callers must call
/// from the UI loop.
pub fn is_app_active() -> bool {
    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        return false;
    };
    objc2_app_kit::NSApplication::sharedApplication(mtm).isActive()
}

/// Seconds since the last system-wide user input (keyboard/mouse). Lets
/// presence gating tell "actively using the machine" from "away from
/// keyboard" without installing an input event tap.
///
/// `None` means the platform could not answer, which is not the same fact as
/// `Some(0.0)` ("input this instant") — a presence gate that conflates them
/// reads an unavailable sensor as a user sitting right there.
#[cfg(not(test))]
pub fn system_idle_seconds() -> Option<f64> {
    // kCGEventSourceStateHIDSystemState = 1; kCGAnyInputEventType = ~0.
    const HID_SYSTEM_STATE: u32 = 1;
    const ANY_INPUT_EVENT: u32 = u32::MAX;
    // SAFETY: `CGEventSourceSecondsSinceLastEventType` is a pointer-free C query
    // over HID state. Both arguments are valid `CGEventSourceStateID` /
    // `CGEventType` values and it returns a plain `CFTimeInterval` (f64 seconds);
    // there is no ownership transfer to manage.
    Some(unsafe { CGEventSourceSecondsSinceLastEventType(HID_SYSTEM_STATE, ANY_INPUT_EVENT) })
}

#[cfg(not(test))]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: u32, event_type: u32) -> f64;
}

/// Apply a new attention request, replacing any prior pending request.
///
/// Must be called on the main thread (`NSApplication` is main-thread-only).
pub fn apply(kind: AttentionKind) {
    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        debug_assert!(false, "request_user_attention called off the main thread");
        return;
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);

    // Cancel any prior request before issuing a new one so the dock
    // settles deterministically when, e.g., the shell pings Critical
    // and immediately Once after.
    cancel_pending(&app);

    match kind {
        AttentionKind::Cancel => { /* already cancelled above */ }
        AttentionKind::Critical => {
            let id = app
                .requestUserAttention(objc2_app_kit::NSRequestUserAttentionType::CriticalRequest);
            store_id(id);
        }
        AttentionKind::Once => {
            let id = app.requestUserAttention(
                objc2_app_kit::NSRequestUserAttentionType::InformationalRequest,
            );
            store_id(id);
        }
    }
}

fn last_request_slot() -> &'static std::sync::Mutex<Option<isize>> {
    static SLOT: std::sync::Mutex<Option<isize>> = std::sync::Mutex::new(None);
    &SLOT
}

fn store_id(id: isize) {
    if let Ok(mut slot) = last_request_slot().lock() {
        *slot = Some(id);
    }
}

fn cancel_pending(app: &objc2_app_kit::NSApplication) {
    let id = match last_request_slot().lock() {
        Ok(mut slot) => slot.take(),
        Err(_) => None,
    };
    if let Some(id) = id {
        app.cancelUserAttentionRequest(id);
    }
}
