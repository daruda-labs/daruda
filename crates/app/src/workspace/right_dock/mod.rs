//! Right-dock four-tab panel (Usage / Skills / Tools / Tasks).

use crate::ui::theme;
use daruda_store::project::RightDockView;
use gpui::{AnyElement, App, Context, IntoElement, ScrollHandle, div, prelude::*, px};

use super::layout::Dock;
use super::layout::RightDockSnapshot;

pub(in crate::workspace) mod mcp_ops;
pub(in crate::workspace) mod skill_ops;
pub(in crate::workspace) mod skills;
pub(in crate::workspace) mod status_pill;
pub(in crate::workspace) mod task_ops;
pub(in crate::workspace) mod task_picker_modal;
pub(in crate::workspace) mod task_workflow_ops;
pub(in crate::workspace) mod tasks;
pub(in crate::workspace) mod tasks_ops;
pub(in crate::workspace) mod tools;
pub(in crate::workspace) mod usage;
pub(in crate::workspace) mod view_tabs;

/// Shared scaffold for a right-dock tab body: a vertical flex column
/// with the panel's standard padding and section gap. Every right-dock
/// view (Usage / Skills / Tasks / Tools) builds its body on this so the
/// outer padding and inter-section spacing have a single definition
/// rather than four near-identical copies.
pub(in crate::workspace) fn right_panel_body() -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .px(px(theme::RIGHT_PANEL_PAD_X))
        .py(px(theme::RIGHT_PANEL_PAD_Y))
        .gap(px(theme::RIGHT_PANEL_SECTION_GAP))
}

/// Render the body for the currently-active right-panel tab.
///
/// Wraps every per-tab body in a vertical scroll container so a long
/// list (e.g. the marketplace catalog inside Skills, or the per-session
/// rows in Usage) doesn't push the dock footer off-screen. The outer
/// dock wrapper sets `overflow_hidden`, which would otherwise clip
/// without offering any way to scroll. A scrollbar thumb overlay sits
/// on top of the scroll viewport so the user can see scroll position.
pub(in crate::workspace) fn render(snap: &RightDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    let body = match snap.right_dock_view {
        RightDockView::Usage => usage::render(snap, cx),
        RightDockView::Skills => skills::render(snap, cx),
        RightDockView::Tasks => tasks::render(snap, cx),
        RightDockView::Tools => tools::render(snap, cx),
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
    let t = theme::current(cx);
    crate::ui::scrollbar::vertical_thumb(
        "right-panel-scrollbar-thumb",
        viewport_h,
        viewport_h + max_offset,
        handle.offset().y,
        px(0.),
        t.right_panel_scrollbar_thumb,
        t.right_panel_scrollbar_thumb_hover,
    )
}
