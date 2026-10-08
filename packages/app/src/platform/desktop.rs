//! Desktop lifecycle capabilities outside GPUI's portable window interface.

#[cfg(windows)]
#[path = "desktop_windows.rs"]
mod windows;

pub(crate) fn install(cx: &mut gpui::App) {
    #[cfg(windows)]
    windows::install(cx);
    #[cfg(not(windows))]
    let _ = cx;
}

pub(crate) fn activate(window: &gpui::Window) {
    #[cfg(windows)]
    windows::show(window);
    window.activate_window();
}

/// Reveal a notification target, or the focused workspace with a stable fallback.
pub(crate) fn reveal(target: Option<super::notifications::Target>, cx: &mut gpui::App) {
    use crate::window_registry::WindowRegistry;
    let handle = target
        .and_then(|target| WindowRegistry::workspace_window_for_uuid(target.workspace, cx))
        .or_else(|| WindowRegistry::active_workspace_handle(cx))
        .or_else(|| WindowRegistry::first_workspace(cx).map(|(handle, _)| handle));
    let mut activated = false;
    WindowRegistry::for_each_workspace(cx, |ws, window, cx| {
        if !activated && Some(window.window_handle()) == handle {
            if let Some(target) = target.filter(|target| target.workspace == ws.uuid()) {
                ws.reveal_notification_pane(target.pane, window, cx);
            }
            activate(window);
            activated = true;
        }
    });
}

pub(crate) fn hide_to_tray(window: &gpui::Window, cx: &gpui::App) -> bool {
    #[cfg(windows)]
    return windows::hide_to_tray(window, cx);
    #[cfg(not(windows))]
    {
        let _ = (window, cx);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext as _;

    #[gpui::test]
    fn reveal_prefers_the_notification_target_and_preserves_the_active_fallback(
        cx: &mut gpui::TestAppContext,
    ) {
        let first = crate::test_support::workspace_for_control(cx);
        let second = crate::test_support::workspace_for_control(cx);
        let target = super::super::notifications::Target {
            workspace: first.workspace.read_with(cx, |ws, _| ws.uuid()),
            pane: u64::MAX,
        };
        cx.update_window(second.window.into(), |_, window, _| {
            window.activate_window()
        })
        .unwrap();
        cx.update(|cx| {
            reveal(None, cx);
            assert_eq!(cx.active_window(), Some(second.window.into()));
            reveal(Some(target), cx);
            assert_eq!(cx.active_window(), Some(first.window.into()));
        });
        cx.update_window(second.window.into(), |_, window, _| {
            window.activate_window()
        })
        .unwrap();
        cx.update(|cx| {
            crate::window_registry::WindowRegistry::deregister(&first.workspace.downgrade(), cx);
            reveal(Some(target), cx);
            assert_eq!(cx.active_window(), Some(second.window.into()));
        });
    }
}
