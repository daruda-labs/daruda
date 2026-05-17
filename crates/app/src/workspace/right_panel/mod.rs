//! Right-dock four-tab panel (Usage / Skills / Tools / Tasks).

use crate::ui::theme;
use daruda_store::project::RightSidebarView;
use gpui::{AnyElement, App, Context, IntoElement, ScrollHandle, div, prelude::*, px};

use super::dock::Dock;
use super::dock_snap::RightSidebarSnapshot;

pub(in crate::workspace) mod skills;
pub(in crate::workspace) mod status_pill;
pub(in crate::workspace) mod task_picker_modal;
pub(in crate::workspace) mod tasks;
pub(in crate::workspace) mod tools;
pub(in crate::workspace) mod usage;
pub(in crate::workspace) mod view_tabs;

/// Render the body for the currently-active right-panel tab.
///
/// Wraps every per-tab body in a vertical scroll container so a long
/// list (e.g. the marketplace catalog inside Skills, or the per-session
/// rows in Usage) doesn't push the dock footer off-screen. The outer
/// dock wrapper sets `overflow_hidden`, which would otherwise clip
/// without offering any way to scroll. A scrollbar thumb overlay sits
/// on top of the scroll viewport so the user can see scroll position.
pub(in crate::workspace) fn render(
    snap: &RightSidebarSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let body = match snap.right_sidebar_view {
        RightSidebarView::Usage => usage::render(snap, cx),
        RightSidebarView::Skills => skills::render(snap, cx),
        RightSidebarView::Tasks => tasks::render(snap, cx),
        RightSidebarView::Tools => tools::render(snap, cx),
    };
    let handle = snap.right_panel_scroll_handle.clone();
    div()
        .relative()
        .size_full()
        .overflow_hidden()
        .child(
            div()
                .id("right-panel-scroll")
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .overflow_y_scroll()
                .track_scroll(&handle)
                .child(body),
        )
        .children(scrollbar_thumb(&handle, cx))
        .into_any_element()
}

/// Scrollbar thumb overlay for the right-dock body. Returns `None`
/// when content fits without scrolling. Shape and position are derived
/// from the shared `ScrollHandle`'s viewport / max-offset / current
/// offset — the same approach used by `settings_window::render`.
fn scrollbar_thumb(handle: &ScrollHandle, cx: &App) -> Option<AnyElement> {
    let viewport_h = handle.bounds().size.height;
    let max_offset = handle.max_offset().height;
    if max_offset <= px(0.) || viewport_h <= px(0.) {
        return None;
    }
    let content_h = viewport_h + max_offset;
    let thumb_ratio = (viewport_h / content_h).min(1.0_f32);
    let raw_thumb_h = viewport_h * thumb_ratio;
    let thumb_h = raw_thumb_h.max(px(theme::RIGHT_PANEL_SCROLLBAR_MIN_THUMB_H));
    let track_h = viewport_h - thumb_h;
    let scroll_frac = ((-handle.offset().y) / max_offset).clamp(0.0_f32, 1.0_f32);
    let thumb_top = track_h * scroll_frac;
    let w = px(theme::RIGHT_PANEL_SCROLLBAR_W);
    let r = px(theme::RIGHT_PANEL_SCROLLBAR_MARGIN_R);
    let t = theme::current(cx);
    let thumb_bg = t.right_panel_scrollbar_thumb;
    let thumb_hover_bg = t.right_panel_scrollbar_thumb_hover;
    Some(
        div()
            .id("right-panel-scrollbar-thumb")
            .absolute()
            .top(thumb_top)
            .right(r)
            .w(w)
            .h(thumb_h)
            .rounded(w / 2.0)
            .bg(thumb_bg)
            .hover(move |d| d.bg(thumb_hover_bg))
            .into_any_element(),
    )
}
