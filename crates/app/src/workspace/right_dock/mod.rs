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
pub(in crate::workspace) mod tools;
pub(in crate::workspace) mod usage;
pub(in crate::workspace) mod usage_ops;
pub(in crate::workspace) mod usage_session_ops;
pub(in crate::workspace) mod view_tabs;

/// Shared scaffold for a right-dock tab body: a vertical flex column
/// with the panel's standard padding and section gap, so every view
/// (Usage / Skills / Tasks / Tools) shares one definition.
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
/// list doesn't push the dock footer off-screen (the outer dock wrapper
/// sets `overflow_hidden`), with a scrollbar thumb overlay for position.
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
/// when content fits; shape and position derive from the shared
/// `ScrollHandle`'s viewport / max-offset / current offset.
fn scrollbar_thumb(handle: &ScrollHandle, cx: &App) -> Option<AnyElement> {
    let viewport_h = handle.bounds().size.height;
    let max_offset = handle.max_offset().y;
    let t = theme::current(cx);
    crate::ui::scrollbar::vertical_thumb(
        "right-panel-scrollbar-thumb",
        viewport_h,
        viewport_h + max_offset,
        handle.offset().y,
        px(0.),
        t.scrollbar_thumb,
        t.right_panel_scrollbar_thumb_hover,
    )
}
