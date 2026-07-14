//! `NSApplication -requestUserAttention:` wrapper.
//!
//! macOS exposes two attention levels:
//! - `NSCriticalRequest` — dock icon bounces continuously until the
//!   app receives focus.
//! - `NSInformationalRequest` — single bounce.
//!
//! Each call returns a request ID; cancellation requires that same ID.
//! daruda tracks the most recent ID so an `OSC 1337 ;
//! RequestAttention=no` from any pane clears the bounce. This is a
//! single-shared-slot model — fine for the "one bounce at a time"
//! reality of the dock, and matches iTerm2's
//! `iTermController -cancelUserAttentionRequest`.

use std::sync::Mutex;

use daruda_terminal::AttentionKind;
use objc2_app_kit::{NSApplication, NSRequestUserAttentionType};
use objc2_foundation::MainThreadMarker;

/// True when the daruda window is currently the focused app.
/// Used by notification gating: the "skip the focused pane" rule
/// only applies when daruda itself is foreground — if the user is
/// in another app, every pane's notification is welcome regardless
/// of which one daruda thinks is focused.
///
/// Returns `false` if called off the main thread; callers must call
/// from the UI loop.
pub fn is_app_active() -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    NSApplication::sharedApplication(mtm).isActive()
}

/// Seconds since the last system-wide user input (keyboard/mouse), read from
/// CoreGraphics' HID idle clock. Lets presence gating tell "actively using the
/// machine" from "away from keyboard" without installing an input event tap.
/// Safe to call from any thread (the underlying query reads shared HID state),
/// but callers here invoke it from the GPUI foreground loop.
pub fn system_idle_seconds() -> f64 {
    // kCGEventSourceStateHIDSystemState = 1; kCGAnyInputEventType = ~0.
    const HID_SYSTEM_STATE: u32 = 1;
    const ANY_INPUT_EVENT: u32 = u32::MAX;
    // SAFETY: `CGEventSourceSecondsSinceLastEventType` is a pointer-free C query
    // over HID state. Both arguments are valid `CGEventSourceStateID` /
    // `CGEventType` values and it returns a plain `CFTimeInterval` (f64 seconds);
    // there is no ownership transfer to manage.
    unsafe { CGEventSourceSecondsSinceLastEventType(HID_SYSTEM_STATE, ANY_INPUT_EVENT) }
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: u32, event_type: u32) -> f64;
}

/// Apply a new attention request, replacing any prior pending request.
///
/// Must be called on the main thread (`NSApplication` is main-thread-only).
pub fn apply(kind: AttentionKind) {
    let Some(mtm) = MainThreadMarker::new() else {
        debug_assert!(false, "request_user_attention called off the main thread");
        return;
    };
    let app = NSApplication::sharedApplication(mtm);

    // Cancel any prior request before issuing a new one so the dock
    // settles deterministically when, e.g., the shell pings Critical
    // and immediately Once after.
    cancel_pending(&app);

    match kind {
        AttentionKind::Cancel => { /* already cancelled above */ }
        AttentionKind::Critical => {
            let id = app.requestUserAttention(NSRequestUserAttentionType::CriticalRequest);
            store_id(id);
        }
        AttentionKind::Once => {
            let id = app.requestUserAttention(NSRequestUserAttentionType::InformationalRequest);
            store_id(id);
        }
    }
}

fn last_request_slot() -> &'static Mutex<Option<isize>> {
    static SLOT: Mutex<Option<isize>> = Mutex::new(None);
    &SLOT
}

fn store_id(id: isize) {
    if let Ok(mut slot) = last_request_slot().lock() {
        *slot = Some(id);
    }
}

fn cancel_pending(app: &NSApplication) {
    let id = match last_request_slot().lock() {
        Ok(mut slot) => slot.take(),
        Err(_) => None,
    };
    if let Some(id) = id {
        app.cancelUserAttentionRequest(id);
    }
}
