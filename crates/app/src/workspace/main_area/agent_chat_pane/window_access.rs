//! How an embed reconcile pass reaches the live [`Window`] it needs to build
//! editor entities.

use gpui::{AnyWindowHandle, App, AppContext as _, Window};

/// Where the live [`Window`] comes from for the caller at hand.
///
/// gpui takes a window *out of* `App::windows` for the whole of an update
/// (`update_window_id`'s `cx.windows.get_mut(id)?.take()?`), so re-entering
/// `update_window` from inside that window's own update cycle — which is where
/// every `cx.listener` handler runs — finds nothing and returns "window not
/// found". A caller that already holds the borrow therefore has to hand it over
/// rather than ask for it again; only a caller outside any update cycle (an
/// async ACP event, a global observer) may resolve one from a stored handle.
///
/// Naming both ways in one type is what keeps that distinction at the call site:
/// a new caller has to say which world it is in.
pub(in crate::workspace) enum WindowAccess<'a> {
    /// Already inside the window's update cycle — use this borrow.
    Live(&'a mut Window),
    /// Outside any window update — re-enter by handle.
    ByHandle(AnyWindowHandle),
}

impl WindowAccess<'_> {
    /// Run `f` against a live window. `Err` only when the handle no longer
    /// resolves, which [`Self::Live`] cannot produce.
    pub(in crate::workspace) fn with<R>(
        &mut self,
        cx: &mut App,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> anyhow::Result<R> {
        match self {
            Self::Live(window) => Ok(f(window, cx)),
            Self::ByHandle(handle) => cx.update_window(*handle, |_, window, cx| f(window, cx)),
        }
    }
}

#[cfg(test)]
mod tests;
