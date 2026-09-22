//! The window's title bar row, assembled once for every daruda window.
//!
//! Every window daruda opens shares `build_titlebar_options`, whose
//! `appears_transparent` removes the OS caption off macOS — so the row that
//! carries the drag region and the window controls is a property of the
//! window, not of the `Workspace` that usually fills it. Settings and Welcome
//! call the same builders.

pub(in crate::workspace) mod policy;
mod window_controls;

use gpui::{AnyElement, App, Div, Hsla, Window, div, prelude::*, px};

pub(in crate::workspace) use policy::{FrameFacts, WindowChrome};

use crate::ui::theme;

/// Resolve the chrome for `window` from this build's platform and the live
/// decoration mode. The single `cfg!` read; everything below takes the value.
pub(in crate::workspace) fn chrome_for_window(window: &Window) -> WindowChrome {
    WindowChrome::new(
        FrameFacts {
            os_draws_caption: cfg!(target_os = "macos"),
            server_decorated: matches!(window.window_decorations(), gpui::Decorations::Server),
        },
        cfg!(target_os = "windows"),
    )
}

/// Build the title bar. `trailing` is whatever the host puts to the right of
/// the drag strip — the workspace's dock toggles, nothing for Settings.
pub(in crate::workspace) fn render(
    chrome: WindowChrome,
    bg: Hsla,
    leading: Option<AnyElement>,
    trailing: Option<AnyElement>,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let inset = if chrome.is_client() {
        theme::CLIENT_CHROME_INSET
    } else {
        theme::TRAFFIC_LIGHT_WIDTH
    };

    div()
        .flex()
        .flex_row()
        .w_full()
        .h(px(theme::TITLE_BAR_HEIGHT))
        .bg(bg)
        .items_center()
        .child(div().flex_none().w(px(inset)))
        .children(leading)
        .child(window_controls::drag_region(chrome.tier, window, cx))
        .children(trailing)
        .when(chrome.is_client(), |d| {
            d.child(window_controls::window_controls(chrome, window, cx))
        })
}

#[cfg(feature = "screenshot")]
impl crate::workspace::Workspace {
    /// Draw this window as if the platform left no caption, so the arm a
    /// macOS host never reaches can still be reviewed in a capture. The
    /// field's only other writer is the constructor.
    pub(in crate::workspace) fn force_client_chrome_for_shot(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.window_chrome.kind = policy::TitleBarChrome::Client;
        cx.notify();
    }
}
