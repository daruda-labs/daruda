//! File-viewer search panel — floating top-right overlay with input,
//! match counter, prev/next/close controls. Styled to match the
//! terminal's scrollback search bar.

use crate::ui::theme;
use gpui::{AnyElement, Context, IntoElement, MouseButton, MouseDownEvent, div, prelude::*, px};

use crate::surface::strings;
use crate::workspace::Workspace;
use crate::workspace::pane_file_view::FileViewerSearch;

/// Floating search panel — top-right corner, below the toolbar.
pub(super) fn render_search_panel(
    search: &FileViewerSearch,
    toolbar_h: gpui::Pixels,
    search_input: gpui::Entity<crate::ui::InputState>,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let has_query = !search.query.is_empty();
    let total = search.matches.len();
    let focused_n = search.focused.map(|i| i + 1).unwrap_or(0);

    let t = theme::current(cx);
    let count_color = t.file_viewer_search_count;
    let empty_color = t.file_viewer_search_empty;
    let text_color = t.file_viewer_search_text;
    let button_color = theme::SEARCH_BUTTON;

    let (counter, counter_color) = if !has_query {
        (String::new(), count_color)
    } else if total == 0 {
        (strings::FILE_VIEWER_SEARCH_NO_MATCH.to_owned(), empty_color)
    } else {
        (format!("{focused_n}/{total}"), count_color)
    };

    // The `search_input` entity is rendered as the panel's text field
    // below; clone it for each closure that mutates the input (clear /
    // close buttons). Cloning an Entity is cheap (refcount).
    let input_for_clear = search_input.clone();
    let input_for_close = search_input.clone();

    div()
        .id("file-viewer-search-panel")
        .absolute()
        .top(toolbar_h + px(theme::FILE_VIEWER_SEARCH_MARGIN_T))
        .right(px(theme::FILE_VIEWER_SEARCH_MARGIN_R))
        .h(px(theme::FILE_VIEWER_SEARCH_PANEL_H))
        .w(px(theme::FILE_VIEWER_SEARCH_PANEL_W))
        .px(px(theme::FILE_VIEWER_SEARCH_PAD_X))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::FILE_VIEWER_SEARCH_ITEM_GAP))
        .bg(theme::SEARCH_PANEL_BG)
        .border_1()
        .border_color(theme::SEARCH_PANEL_BORDER)
        .rounded(px(theme::SEARCH_PANEL_RADIUS))
        .text_size(px(theme::FILE_VIEWER_SEARCH_FONT_SIZE))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()),
        )
        // Search input + match counter
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .flex()
                .items_center()
                .gap(px(theme::FILE_VIEWER_SEARCH_ITEM_GAP))
                .child(div().flex_1().overflow_hidden().child(crate::ui::input(
                    &search_input,
                    cx,
                    (),
                )))
                .when(!counter.is_empty(), |d| {
                    d.child(
                        div()
                            .flex_none()
                            .text_color(counter_color)
                            .text_size(px(theme::FILE_VIEWER_SEARCH_COUNTER_SIZE))
                            .whitespace_nowrap()
                            .child(counter),
                    )
                }),
        )
        // Clear button (only when query is non-empty). The input we
        // clear is the per-pane `FileContent.search_input` passed in
        // through `search_input` — each open file viewer owns its own.
        .when(has_query, |d| {
            let clear_input = input_for_clear;
            d.child(
                div()
                    .id("fv-search-clear")
                    .px(px(theme::FILE_VIEWER_SEARCH_BTN_PAD_X))
                    .text_color(button_color)
                    .cursor_pointer()
                    .hover(|s| s.text_color(text_color))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _: &MouseDownEvent, window, cx| {
                            clear_input
                                .update(cx, |inp, cx_state| inp.set_value("", window, cx_state));
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    )
                    .child(strings::FILE_VIEWER_SEARCH_CLEAR),
            )
        })
        // Prev match button — operates on the focused file pane.
        .child(
            div()
                .id("fv-search-prev")
                .px(px(theme::FILE_VIEWER_SEARCH_BTN_PAD_X))
                .text_color(button_color)
                .cursor_pointer()
                .hover(|s| s.text_color(text_color))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                        if let Some(fv) = this.focused_file_view_mut() {
                            fv.search_prev_match();
                        }
                        this.scroll_file_viewer_to_focused_match();
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
                .child(strings::FILE_VIEWER_SEARCH_PREV),
        )
        // Next match button — operates on the focused file pane.
        .child(
            div()
                .id("fv-search-next")
                .px(px(theme::FILE_VIEWER_SEARCH_BTN_PAD_X))
                .text_color(button_color)
                .cursor_pointer()
                .hover(|s| s.text_color(text_color))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                        if let Some(fv) = this.focused_file_view_mut() {
                            fv.search_next_match();
                        }
                        this.scroll_file_viewer_to_focused_match();
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
                .child(strings::FILE_VIEWER_SEARCH_NEXT),
        )
        // Close button — closes the find panel (not the tab).
        .child({
            let close_input = input_for_close;
            div()
                .id("fv-search-close")
                .ml(px(theme::FILE_VIEWER_SEARCH_BTN_ML))
                .px(px(theme::FILE_VIEWER_SEARCH_BTN_PAD_X))
                .text_color(button_color)
                .cursor_pointer()
                .hover(|s| s.text_color(text_color))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        if let Some(fv) = this.focused_file_view_mut() {
                            fv.search_close();
                        }
                        close_input.update(cx, |inp, cx_state| inp.set_value("", window, cx_state));
                        this.focus_pane(this.focused_pane_id, window, cx);
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
                .child(strings::FILE_VIEWER_SEARCH_CLOSE_BTN)
        })
        .into_any_element()
}
