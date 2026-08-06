use gpui::{AnyWindowHandle, Context, IntoElement, Render, TestAppContext, Window, div};

use super::WindowAccess;

/// Minimal window root — these tests exercise window resolution, not rendering.
struct Empty;

impl Render for Empty {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn empty_window(cx: &mut TestAppContext) -> (gpui::WindowHandle<Empty>, AnyWindowHandle) {
    let window = cx.add_window(|_, _| Empty);
    let handle = AnyWindowHandle::from(window);
    (window, handle)
}

/// The defect this type exists for: inside a window's own update cycle gpui has
/// taken the window out of `App::windows`, so resolving one *by handle* there
/// fails — while the borrow the caller already holds still works.
#[gpui::test]
fn by_handle_fails_inside_the_window_update_cycle_but_live_does_not(cx: &mut TestAppContext) {
    let (window, handle) = empty_window(cx);

    window
        .update(cx, |_, live, cx| {
            assert!(
                WindowAccess::ByHandle(handle).with(cx, |_, _| ()).is_err(),
                "a nested update_window cannot resolve the window it is already inside"
            );
            assert!(
                WindowAccess::Live(live).with(cx, |_, _| ()).is_ok(),
                "the borrow the caller already holds is the way through"
            );
        })
        .expect("the window is open");
}

/// Outside any update cycle the handle resolves — the async ACP-event path.
#[gpui::test]
fn by_handle_resolves_outside_the_update_cycle(cx: &mut TestAppContext) {
    let (_window, handle) = empty_window(cx);

    cx.update(|cx| {
        assert!(WindowAccess::ByHandle(handle).with(cx, |_, _| ()).is_ok());
    });
}
