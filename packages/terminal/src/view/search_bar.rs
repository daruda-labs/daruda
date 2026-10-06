use gpui::{Context, IntoElement, MouseButton, MouseDownEvent, div, prelude::*};

use super::text_edit;
use super::text_metrics;
use super::{SearchClearQuery, SearchClose, SearchNext, SearchPrev, SearchToggleRegex};
use super::{TerminalView, search};

/// Geometry for the floating search bar.
///
/// Every pixel value that shapes the bar lives here. `render_search_bar`
/// passes these fields directly into the GPUI div tree (paddings, gaps,
/// widths), and its input click handler calls `text_left_offset()` —
/// a *derived* value computed from the same fields — to locate the
/// query's first glyph. Adjusting a padding/gap therefore moves both
/// the visual layout and the caret-hit mapping in lockstep; neither
/// side carries its own magic number.
#[derive(Clone, Copy)]
pub(super) struct SearchBarLayout {
    pub panel_width: f32,
    pub panel_right_gap: f32,
    pub panel_margin_top: f32,
    pub panel_pad_x: f32,
    pub panel_pad_y: f32,
    pub panel_gap: f32,
    pub icon_width: f32,
    pub icon_font_size: f32,
    pub input_pad_x: f32,
    pub input_pad_y: f32,
    pub font_size: f32,
}

impl SearchBarLayout {
    pub const DEFAULT: Self = Self {
        panel_width: 380.0,
        panel_right_gap: 12.0,
        panel_margin_top: 8.0,
        panel_pad_x: 10.0,
        panel_pad_y: 6.0,
        panel_gap: 8.0,
        icon_width: 28.0,
        icon_font_size: 22.0,
        input_pad_x: 8.0,
        input_pad_y: 3.0,
        font_size: 13.0,
    };

    pub const fn text_left_offset(&self) -> f32 {
        self.panel_pad_x + self.icon_width + self.panel_gap + self.input_pad_x
    }
}

impl TerminalView {
    pub(super) fn render_search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = SearchBarLayout::DEFAULT;
        let query = self.state.search.query.clone();
        let cursor = self.state.search.cursor_byte.min(query.len());
        let (before, after) = query.split_at(cursor);
        let before = before.to_string();
        let after = after.to_string();

        let total = self.state.search.matches.len();
        let focused = self.state.search.focused.map(|i| i + 1).unwrap_or(0);
        use crate::ux::{strings as ux_strings, theme as ux_theme};
        let (counter, counter_color) = if let search::SearchMode::Regex {
            compile_error: Some(_),
        } = &self.state.search.mode
        {
            (
                ux_strings::search_regex_error(),
                ux_theme::SEARCH_LABEL_ERROR,
            )
        } else if query.is_empty() {
            (String::new(), ux_theme::SEARCH_LABEL_IDLE)
        } else if total == 0 {
            (
                ux_strings::search_no_matches(),
                ux_theme::SEARCH_LABEL_EMPTY,
            )
        } else {
            (format!("{focused}/{total}"), ux_theme::SEARCH_LABEL_COUNTER)
        };

        let panel_bg = ux_theme::SEARCH_PANEL_BG;
        let border_color = ux_theme::SEARCH_PANEL_BORDER;
        let icon_color = ux_theme::SEARCH_ICON;
        let input_bg = ux_theme::SEARCH_INPUT_BG;
        let input_border = ux_theme::SEARCH_INPUT_BORDER;
        let input_color = ux_theme::SEARCH_BAR_INPUT_TEXT;
        let button_color = ux_theme::SEARCH_BUTTON;
        let button_hover_color = ux_theme::SEARCH_BAR_BUTTON_HOVER;

        let has_query = !query.is_empty();

        div()
            .absolute()
            .top_0()
            .right_0()
            .mt(gpui::px(l.panel_margin_top))
            .mr(gpui::px(l.panel_right_gap))
            .w(gpui::px(l.panel_width))
            .px(gpui::px(l.panel_pad_x))
            .py(gpui::px(l.panel_pad_y))
            .flex()
            .items_center()
            .gap(gpui::px(l.panel_gap))
            .bg(panel_bg)
            .border_1()
            .border_color(border_color)
            .rounded(gpui::px(ux_theme::SEARCH_PANEL_RADIUS))
            .text_size(gpui::px(l.font_size))
            .line_height(gpui::px(l.font_size * 1.2))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .id("search-mode-icon")
                    .w(gpui::px(l.icon_width))
                    .text_size(gpui::px(l.icon_font_size))
                    .text_color(icon_color)
                    .cursor_pointer()
                    .hover(|s| s.text_color(button_hover_color))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.on_search_toggle_regex(&SearchToggleRegex, window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child(if self.state.search.is_regex() {
                        ".*"
                    } else {
                        "⌕"
                    }),
            )
            .child(
                div()
                    .id("search-input")
                    .flex()
                    .flex_1()
                    .items_center()
                    .gap(gpui::px(ux_theme::SEARCH_BAR_INPUT_GAP))
                    .px(gpui::px(l.input_pad_x))
                    .py(gpui::px(l.input_pad_y))
                    .bg(input_bg)
                    .border_1()
                    .border_color(input_border)
                    .rounded(gpui::px(ux_theme::SEARCH_BAR_INPUT_RADIUS))
                    .text_color(input_color)
                    .cursor_text()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            if !this.state.search_overlay {
                                return;
                            }
                            cx.stop_propagation();
                            let Some(bounds) = this.state.last_bounds else {
                                this.state.search.cursor_byte = this.state.search.query.len();
                                cx.notify();
                                return;
                            };
                            let panel_right = bounds.right() - gpui::px(l.panel_right_gap);
                            let panel_left = panel_right - gpui::px(l.panel_width);
                            let text_left = panel_left + gpui::px(l.text_left_offset());
                            let offset_px = ev.position.x - text_left;
                            let byte_idx = text_metrics::byte_index_for_x_in_text(
                                window,
                                &this.state.font,
                                gpui::px(l.font_size),
                                &this.state.search.query,
                                offset_px,
                            )
                            .unwrap_or(this.state.search.query.len());
                            this.state.search.cursor_byte = text_edit::clamp_to_char_boundary(
                                &this.state.search.query,
                                byte_idx,
                            );
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(gpui::px(0.0))
                            .flex()
                            .items_center()
                            .child(div().child(before))
                            .child(
                                div()
                                    .w(gpui::px(ux_theme::SEARCH_BAR_CARET_W))
                                    .h(gpui::px(ux_theme::SEARCH_BAR_CARET_H))
                                    .bg(input_color),
                            )
                            .child(div().child(after)),
                    )
                    .when(!counter.is_empty(), |el| {
                        el.child(
                            div()
                                .text_color(counter_color)
                                .text_size(gpui::px(ux_theme::SEARCH_BAR_COUNTER_FONT_SIZE))
                                .whitespace_nowrap()
                                .child(counter),
                        )
                    }),
            )
            .when(has_query, |el| {
                el.child(
                    div()
                        .id("search-clear")
                        .px(gpui::px(ux_theme::SEARCH_BAR_BUTTON_PAD_X))
                        .text_color(button_color)
                        .hover(|s| s.text_color(button_hover_color))
                        .cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.on_search_clear_query(&SearchClearQuery, window, cx);
                                cx.stop_propagation();
                            }),
                        )
                        .child("✕"),
                )
            })
            .child(
                div()
                    .id("search-prev-btn")
                    .px(gpui::px(ux_theme::SEARCH_BAR_BUTTON_PAD_X))
                    .text_color(button_color)
                    .hover(|s| s.text_color(button_hover_color))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.on_search_prev(&SearchPrev, window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child("◀"),
            )
            .child(
                div()
                    .id("search-next-btn")
                    .px(gpui::px(ux_theme::SEARCH_BAR_BUTTON_PAD_X))
                    .text_color(button_color)
                    .hover(|s| s.text_color(button_hover_color))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.on_search_next(&SearchNext, window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child("▶"),
            )
            .child(
                div()
                    .id("search-close-btn")
                    .ml(gpui::px(ux_theme::SEARCH_BAR_CLOSE_ML))
                    .px(gpui::px(ux_theme::SEARCH_BAR_BUTTON_PAD_X))
                    .text_color(button_color)
                    .hover(|s| s.text_color(button_hover_color))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.on_search_close(&SearchClose, window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child("✕"),
            )
    }

    pub(super) fn recompute_search_matches(&mut self) {
        self.recompute_search_matches_with(true);
    }

    pub(super) fn recompute_search_matches_with(&mut self, reset_focus: bool) {
        if self.state.search.query.is_empty() {
            self.state.search.matches.clear();
            self.state.search.focused = None;
            return;
        }

        // Remember the prior focus by (row, start_col) so a viewport
        // refresh (reset_focus = false) keeps highlighting the same
        // hit even after the match list is rebuilt. Cycling relies on
        // each (row, start_col) being unique — see
        // `multiple_matches_in_same_line_have_distinct_start_cols`.
        let previous_focused_key = self.state.search.focused.and_then(|i| {
            self.state
                .search
                .matches
                .get(i)
                .map(|m| (m.row, m.start_col))
        });

        let result = search::scan_search_matches(
            &self.session,
            &self.state.search.query,
            self.state.search.case_insensitive,
            self.state.search.is_regex(),
        );
        if let search::SearchMode::Regex { compile_error } = &mut self.state.search.mode {
            *compile_error = result.compile_error;
        }
        self.state.search.matches = result.matches;

        self.state.search.focused = if self.state.search.matches.is_empty() {
            None
        } else if reset_focus {
            Some(0)
        } else {
            previous_focused_key
                .and_then(|(row, col)| {
                    self.state
                        .search
                        .matches
                        .iter()
                        .position(|m| m.row == row && m.start_col == col)
                })
                .or(Some(0))
        };
    }
}
