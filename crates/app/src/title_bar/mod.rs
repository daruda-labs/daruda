//! The window's title bar row, assembled once for every daruda window.
//!
//! Every window daruda opens shares `build_titlebar_options`, whose
//! `appears_transparent` removes the OS caption off macOS — so the row that
//! carries the drag region and the window controls is a property of the
//! window, not of the `Workspace` that usually fills it. Settings and Welcome
//! call the same builders.

mod app_menu;
pub(crate) mod policy;
pub(crate) mod window_controls;

use gpui::{AnyElement, App, Div, Hsla, Window, div, prelude::*, px};

pub(crate) use policy::{FrameFacts, WindowChrome};

use crate::ui::theme;

/// Set once by the `client-chrome` capture scenario: report every window as
/// undecorated so the arm a macOS host resolves away can be reviewed.
#[cfg(feature = "screenshot")]
static FORCE_CLIENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Draw every window from here on as if the platform left no caption.
#[cfg(feature = "screenshot")]
pub(crate) fn force_client_chrome_for_shot() {
    FORCE_CLIENT.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(feature = "screenshot")]
fn forced_client() -> bool {
    FORCE_CLIENT.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(not(feature = "screenshot"))]
const fn forced_client() -> bool {
    false
}

/// Resolve the chrome for `window` from this build's platform and the live
/// decoration mode. The one place either fact is read, so every window agrees
/// and a compositor that switches decoration modes is picked up on the next
/// frame rather than frozen at construction.
/// Whether asking the window who draws its frame means anything here.
///
/// Only the Linux backends implement `window_decorations`; everything else
/// inherits the trait default `Decorations::Server`. Reading that on Windows
/// says "the compositor draws our controls" about a window whose caption
/// `appears_transparent` just removed, which switches the whole feature off
/// on the platform it exists for. zed gates the same check on Linux alone.
const fn decorations_are_negotiated() -> bool {
    cfg!(any(target_os = "linux", target_os = "freebsd"))
}

pub(crate) fn chrome_for_window(window: &Window) -> WindowChrome {
    let server_decorated = !forced_client()
        && decorations_are_negotiated()
        && matches!(window.window_decorations(), gpui::Decorations::Server);
    WindowChrome::new(
        FrameFacts {
            os_draws_caption: !forced_client() && cfg!(target_os = "macos"),
            server_decorated,
        },
        cfg!(target_os = "windows"),
    )
}

/// Build the title bar. `trailing` is whatever the host puts to the right of
/// the drag strip — the workspace's dock toggles, nothing for Settings.
pub(crate) fn render(
    chrome: WindowChrome,
    bg: Hsla,
    leading: Option<AnyElement>,
    trailing: Option<AnyElement>,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    div()
        .flex()
        .flex_row()
        .w_full()
        .h(px(theme::TITLE_BAR_HEIGHT))
        // A control pushed past the right edge by a narrow window would be
        // unreachable; clipping keeps it out of the pane below.
        .overflow_hidden()
        .bg(bg)
        .items_center()
        .child(div().flex_none().w(px(chrome.leading_inset())))
        .when(chrome.is_client(), |d| {
            d.children(app_menu::app_menu_button(cx).map(IntoElement::into_any_element))
        })
        .children(leading)
        // Only the client arm gets a drag strip. Where the OS or the
        // compositor still owns the frame it already handles the gesture,
        // and ours would override the platform's own answer to a title-bar
        // double-click.
        .map(|d| {
            if chrome.is_client() {
                d.child(window_controls::drag_region(chrome.tier, window, cx))
            } else {
                d.child(div().flex_1())
            }
        })
        .children(trailing)
        .when(chrome.is_client(), |d| {
            d.child(window_controls::window_controls(chrome, window, cx))
        })
}
