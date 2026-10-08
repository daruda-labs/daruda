//! What a workspace knows about the window it lives in. Gone with the
//! window; only `user_label` and `cached_bounds` are persisted.

use gpui::{AnyWindowHandle, Context, FocusHandle, SharedString, Window};

use super::Workspace;

pub(in crate::workspace) struct WindowRuntime {
    /// Handle to this workspace's GPUI window. Stored so that methods
    /// called from `observe_global` (which has no `&mut Window`) can
    /// re-enter the window via `cx.update_window` when they need to
    /// update widgets whose setters require `&mut Window`.
    pub(in crate::workspace) handle: AnyWindowHandle,
    /// The workspace root's focus handle.
    pub(in crate::workspace) focus_handle: FocusHandle,
    /// Whether this window is in the foreground. Written only by
    /// `set_window_active`.
    pub(in crate::workspace) active: bool,
    /// User-set window title (Window > Edit Window Title…). When `Some`,
    /// replaces the auto-derived `"<pane title> — <cwd>"` string passed to
    /// `window.set_window_title`. Persisted to `ProjectState.window_user_label`.
    pub(in crate::workspace) user_label: Option<SharedString>,
    /// Most recently observed window bounds (position + size). Updated by
    /// the `observe_window_bounds` callback so `save_state()` can persist
    /// the live window geometry without taking `Window` as a parameter.
    pub(in crate::workspace) cached_bounds: Option<daruda_store::project::WindowState>,
    /// Re-entry guard for the platform `on_window_should_close` callback.
    /// Set while the batch close prompt awaits the user's answer; cleared
    /// once the answer lands or the workspace is dropped.
    pub(in crate::workspace) close_in_flight: bool,
}

impl WindowRuntime {
    pub(in crate::workspace) fn new(window: &Window, cx: &mut Context<Workspace>) -> Self {
        Self {
            handle: window.window_handle(),
            focus_handle: cx.focus_handle(),
            active: window.is_window_active(),
            user_label: None,
            cached_bounds: None,
            close_in_flight: false,
        }
    }
}
