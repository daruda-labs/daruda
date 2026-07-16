//! Cross-platform "get the user's attention" + "is daruda foreground" +
//! "how long has the user been away" queries.
//!
//! macOS uses `NSApplication -requestUserAttention:` (dock bounce). Linux has
//! no single vendor API for any of this — it goes through the window
//! manager's EWMH hints over X11 (works under XWayland too, which nearly
//! every Wayland desktop still runs for legacy X11 client support). A pure
//! Wayland session with no reachable X server has no substitute today; the
//! Linux implementation degrades to the same "unavailable" values a caller
//! already has to handle (mirrors the Keychain no-op convention in
//! `telegram::keychain`).
//!
//! macOS:
//! - `NSCriticalRequest` — dock icon bounces continuously until the app
//!   receives focus.
//! - `NSInformationalRequest` — single bounce.
//!
//! Each call returns a request ID; cancellation requires that same ID.
//! daruda tracks the most recent ID so an `OSC 1337 ;
//! RequestAttention=no` from any pane clears the bounce. This is a
//! single-shared-slot model — fine for the "one bounce at a time"
//! reality of the dock, and matches iTerm2's
//! `iTermController -cancelUserAttentionRequest`.

use daruda_terminal::AttentionKind;

/// True when the daruda window is currently the focused app.
/// Used by notification gating: the "skip the focused pane" rule
/// only applies when daruda itself is foreground — if the user is
/// in another app, every pane's notification is welcome regardless
/// of which one daruda thinks is focused.
///
/// Returns `false` if called off the main thread; callers must call
/// from the UI loop.
#[cfg(target_os = "macos")]
pub fn is_app_active() -> bool {
    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        return false;
    };
    objc2_app_kit::NSApplication::sharedApplication(mtm).isActive()
}

/// Linux: no `Window` handle reaches most call sites (e.g. the periodic
/// Telegram-defer pump), so this can't piggyback on gpui's own
/// `Window::is_window_active` the way a render-path call could — it stays a
/// self-contained, zero-argument OS query like the macOS path, using EWMH's
/// `_NET_ACTIVE_WINDOW` root property (the X11 analogue of "which app is
/// frontmost"). `false` when no X11 connection is reachable (pure Wayland, no
/// XWayland) or the active window isn't one of ours.
#[cfg(target_os = "linux")]
pub fn is_app_active() -> bool {
    let Some(state) = linux_x11::state() else {
        return false;
    };
    let Some(active) = linux_x11::active_window(state) else {
        return false;
    };
    linux_x11::window_pid(state, active) == Some(std::process::id())
}

/// Seconds since the last system-wide user input (keyboard/mouse). Lets
/// presence gating tell "actively using the machine" from "away from
/// keyboard" without installing an input event tap.
#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: u32, event_type: u32) -> f64;
}

/// Linux: `XScreenSaverQueryInfo` via the X11 `screensaver` extension.
/// `0.0` (never idle) when no X11 connection is reachable.
#[cfg(target_os = "linux")]
pub fn system_idle_seconds() -> f64 {
    use x11rb::protocol::screensaver::ConnectionExt as _;

    let Some(state) = linux_x11::state() else {
        return 0.0;
    };
    let Some(reply) = state
        .conn
        .screensaver_query_info(state.root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return 0.0;
    };
    linux_x11::ms_to_seconds(reply.ms_since_user_input)
}

/// Apply a new attention request, replacing any prior pending request.
///
/// Must be called on the main thread (`NSApplication` is main-thread-only).
#[cfg(target_os = "macos")]
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

/// Linux: EWMH `_NET_WM_STATE_DEMANDS_ATTENTION`, toggled on every one of
/// this process's top-level windows (found via `_NET_CLIENT_LIST` +
/// `_NET_WM_PID`, the same lookup `is_app_active` uses). EWMH's flag is a
/// plain boolean — unlike macOS there's no continuous-vs-single-bounce
/// distinction, so `Critical` and `Once` both set it and `Cancel` clears it.
/// No-op when no X11 connection is reachable.
#[cfg(target_os = "linux")]
pub fn apply(kind: AttentionKind) {
    let Some(state) = linux_x11::state() else {
        return;
    };
    let add = !matches!(kind, AttentionKind::Cancel);
    for window in linux_x11::own_windows(state) {
        linux_x11::set_demands_attention(state, window, add);
    }
}

#[cfg(target_os = "macos")]
fn last_request_slot() -> &'static std::sync::Mutex<Option<isize>> {
    static SLOT: std::sync::Mutex<Option<isize>> = std::sync::Mutex::new(None);
    &SLOT
}

#[cfg(target_os = "macos")]
fn store_id(id: isize) {
    if let Ok(mut slot) = last_request_slot().lock() {
        *slot = Some(id);
    }
}

#[cfg(target_os = "macos")]
fn cancel_pending(app: &objc2_app_kit::NSApplication) {
    let id = match last_request_slot().lock() {
        Ok(mut slot) => slot.take(),
        Err(_) => None,
    };
    if let Some(id) = id {
        app.cancelUserAttentionRequest(id);
    }
}

/// Shared X11 connection + EWMH atom lookups backing the Linux
/// implementations above. One lazily-initialized connection for the whole
/// process — mirrors the macOS side's single shared `last_request_slot`.
#[cfg(target_os = "linux")]
mod linux_x11 {
    use std::sync::OnceLock;

    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        self, Atom, AtomEnum, ClientMessageData, ClientMessageEvent, ConnectionExt as _, EventMask,
        Window,
    };
    use x11rb::rust_connection::RustConnection;

    x11rb::atom_manager! {
        pub Atoms: AtomsCookie {
            _NET_ACTIVE_WINDOW,
            _NET_CLIENT_LIST,
            _NET_WM_PID,
            _NET_WM_STATE,
            _NET_WM_STATE_DEMANDS_ATTENTION,
        }
    }

    pub struct X11State {
        pub conn: RustConnection,
        pub root: Window,
        pub atoms: Atoms,
    }

    /// The shared connection, or `None` once a connect attempt has failed
    /// (pure Wayland with no reachable XWayland). Cached rather than retried
    /// per call — a missing X server doesn't come back mid-process.
    pub fn state() -> Option<&'static X11State> {
        static STATE: OnceLock<Option<X11State>> = OnceLock::new();
        STATE.get_or_init(connect).as_ref()
    }

    fn connect() -> Option<X11State> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen_num)?.root;
        let atoms = Atoms::new(&conn).ok()?.reply().ok()?;
        Some(X11State { conn, root, atoms })
    }

    /// `_NET_ACTIVE_WINDOW` off the root window — the window the WM
    /// currently considers focused, or `None` if absent/unset (id `0`).
    pub fn active_window(state: &X11State) -> Option<Window> {
        let window = read_property_u32(state, state.root, state.atoms._NET_ACTIVE_WINDOW)?;
        (window != 0).then_some(window)
    }

    /// `_NET_WM_PID` on `window` — the owning process's pid, if the window
    /// (and its window manager) advertises one.
    pub fn window_pid(state: &X11State, window: Window) -> Option<u32> {
        read_property_u32(state, window, state.atoms._NET_WM_PID)
    }

    /// Every top-level window (from `_NET_CLIENT_LIST`) owned by this
    /// process, identified by matching `_NET_WM_PID` against `process::id()`.
    pub fn own_windows(state: &X11State) -> Vec<Window> {
        let my_pid = std::process::id();
        let Some(reply) = xproto::get_property(
            &state.conn,
            false,
            state.root,
            state.atoms._NET_CLIENT_LIST,
            AtomEnum::WINDOW,
            0,
            u32::MAX,
        )
        .ok()
        .and_then(|cookie| cookie.reply().ok()) else {
            return Vec::new();
        };
        let Some(windows) = reply.value32() else {
            return Vec::new();
        };
        windows
            .filter(|&w| window_pid(state, w) == Some(my_pid))
            .collect()
    }

    /// Request the window manager add/remove `_NET_WM_STATE_DEMANDS_ATTENTION`
    /// on `window`, per the EWMH `_NET_WM_STATE` client-message protocol (the
    /// property itself is WM-owned — a client must ask via message, not write
    /// it directly).
    pub fn set_demands_attention(state: &X11State, window: Window, add: bool) {
        const NET_WM_STATE_REMOVE: u32 = 0;
        const NET_WM_STATE_ADD: u32 = 1;
        const SOURCE_INDICATION_NORMAL: u32 = 1;

        let action = if add {
            NET_WM_STATE_ADD
        } else {
            NET_WM_STATE_REMOVE
        };
        let data = ClientMessageData::from([
            action,
            state.atoms._NET_WM_STATE_DEMANDS_ATTENTION,
            0,
            SOURCE_INDICATION_NORMAL,
            0,
        ]);
        let event = ClientMessageEvent::new(32, window, state.atoms._NET_WM_STATE, data);
        let _ = state.conn.send_event(
            false,
            state.root,
            EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
            event,
        );
        let _ = state.conn.flush();
    }

    fn read_property_u32(state: &X11State, window: Window, property: Atom) -> Option<u32> {
        let reply = xproto::get_property(&state.conn, false, window, property, AtomEnum::ANY, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        reply.value32()?.next()
    }

    /// `XScreenSaverQueryInfo`'s `ms_since_user_input`, converted to the
    /// seconds unit `system_idle_seconds` reports.
    pub fn ms_to_seconds(ms: u32) -> f64 {
        f64::from(ms) / 1000.0
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn ms_to_seconds_converts() {
            assert_eq!(ms_to_seconds(0), 0.0);
            assert!((ms_to_seconds(1500) - 1.5).abs() < f64::EPSILON);
            assert!((ms_to_seconds(60_000) - 60.0).abs() < f64::EPSILON);
        }
    }
}
