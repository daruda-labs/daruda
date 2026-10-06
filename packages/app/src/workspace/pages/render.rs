//! Page navigation and the central page shell reuse the staged dock data.

use gpui::{AnyElement, App, IntoElement, div, prelude::*, px};

use super::{Page, PageState};
use crate::surface::strings;
use crate::ui::{ButtonVariants as _, button_icon, button_with_icon, icons, theme};
use crate::workspace::layout::{LeftDockSnapshot, RightDockSnapshot};
use crate::workspace::right_dock;

pub(in crate::workspace) fn navigation(snap: &LeftDockSnapshot, cx: &App) -> AnyElement {
    let t = theme::current(cx);
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(theme::DOCK_NAV_ROW_GAP))
        .px(px(theme::DOCK_NAV_PAD_X))
        .py(px(theme::DOCK_NAV_PAD_Y))
        .border_b_1()
        .border_color(t.border)
        .children([Page::Tasks, Page::Flows].into_iter().map(|page| {
            let workspace = snap.workspace.clone();
            button_with_icon(page.icon(), page.label(), page.icon())
                .ghost()
                .w_full()
                .justify_start()
                .h(px(theme::DOCK_NAV_ROW_HEIGHT))
                .text_size(px(theme::LANE_LABEL_FONT_SIZE))
                .when(snap.workspace_page == Some(page), |button| {
                    button.bg(t.lane_card_active_bg).text_color(t.text_primary)
                })
                .on_click(move |_, window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| ws.open_page(page, window, cx));
                    }
                })
        }))
        .into_any_element()
}

pub(in crate::workspace) fn content(
    state: &PageState,
    snap: &RightDockSnapshot,
    cx: &App,
) -> AnyElement {
    let close = close_button((*snap.workspace).clone(), cx);
    frame(
        state,
        right_dock::tasks::header(snap, close, cx),
        right_dock::tasks::render(snap, cx),
        cx,
    )
}

pub(in crate::workspace) fn flow_content(
    state: &PageState,
    snap: &crate::workspace::flow_browser::FlowPageSnapshot,
    cx: &App,
) -> AnyElement {
    let close = close_button(snap.workspace.clone(), cx);
    frame(
        state,
        super::flows::header(snap, close, cx),
        super::flows::render(snap, cx),
        cx,
    )
}

fn close_button(workspace: gpui::WeakEntity<crate::workspace::Workspace>, cx: &App) -> AnyElement {
    button_icon("close-workspace-page", icons::CLOSE, cx)
        .debug_selector(|| "close-workspace-page".into())
        .tooltip(strings::common::btn_close())
        .on_click(move |_, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.return_to_worktree(window, cx));
            }
        })
        .into_any_element()
}

fn frame(state: &PageState, header: AnyElement, body: AnyElement, cx: &App) -> AnyElement {
    let t = theme::current(cx);
    div()
        .flex()
        .flex_col()
        .size_full()
        .min_w_0()
        .bg(t.dock_bg)
        .child(div().border_b_1().border_color(t.border).child(header))
        .child(
            div()
                .id("workspace-page-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&state.scroll)
                .child(
                    div()
                        .mx_auto()
                        .w_full()
                        .max_w(px(theme::DOCK_PAGE_MAX_WIDTH))
                        .p(px(theme::DOCK_PAGE_PAD))
                        .child(body),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::Page;

    #[test]
    fn navigation_artwork_distinguishes_the_pages() {
        assert_ne!(Page::Tasks.icon(), Page::Flows.icon());
    }
}
