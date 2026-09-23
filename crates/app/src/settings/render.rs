//! `impl Render for SettingsView` — sidebar + body layout.

use crate::ui::theme;
use daruda_config::BuiltinSection;
use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, KeyDownEvent, Render, Window, div, prelude::*, px,
};

use super::{SettingsView, navigation, settings_button as button};
use crate::surface::strings as s;

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_bg = theme::current(cx).welcome_bg;

        let query = self
            .sidebar_search_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let results = super::search::query(&query);
        let body = if query.is_empty() {
            self.render_section_body(cx)
        } else {
            self.render_search_results(&query, &results, cx)
        };
        let sidebar = self.render_sidebar_nav(&query, &results, cx);

        // Both, not one or the other: a conflict is a standing question about a
        // field, an error is the report on the action just taken. Rendering the
        // conflict *instead of* the error silently dropped the second.
        let mut body_with_error = div()
            .flex()
            .flex_col()
            .min_w_0()
            .w_full()
            .when(
                !query.is_empty() || !navigation::is_catalog(self.active_section),
                |el| el.max_w(px(theme::SETTINGS_CONTENT_MAX_W)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::PAD_SM))
                    .mb(px(theme::SETTINGS_GROUP_GAP))
                    .child(
                        div()
                            .text_size(px(theme::MODAL_TITLE_FONT_SIZE))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme::current(cx).text_primary)
                            .child(if query.is_empty() {
                                navigation::label(self.active_section)
                            } else {
                                s::settings_search_heading(&query)
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                            .text_color(theme::current(cx).text_muted)
                            .child(if query.is_empty() {
                                navigation::description(self.active_section)
                            } else {
                                s::settings_search_count(results.len())
                            }),
                    ),
            );
        if let Some(err) = self.error.as_ref() {
            body_with_error = body_with_error.child(
                div()
                    .pb(px(theme::MODAL_PANEL_GAP))
                    .child(crate::ui::alert::error("settings-error", err.clone())),
            );
        }
        if let Some(conflict) = self.conflict.as_ref() {
            body_with_error = body_with_error.child(
                div()
                    .pb(px(theme::MODAL_PANEL_GAP))
                    .flex()
                    .flex_col()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .child(crate::ui::alert::warning(
                        "settings-conflict",
                        s::settings_external_change(conflict.field().path()),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap(px(theme::MODAL_FOOTER_GAP))
                            .child(
                                button(
                                    "settings-conflict-reload",
                                    s::settings_use_external_value(),
                                )
                                .tab_stop(true)
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, window, cx| {
                                        this.reload_conflict(window, cx);
                                    },
                                )),
                            )
                            .child(
                                button(
                                    "settings-conflict-overwrite",
                                    s::settings_overwrite_external_value(),
                                )
                                .tab_stop(true)
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, window, cx| {
                                        this.overwrite_conflict(window, cx);
                                    },
                                )),
                            ),
                    ),
            );
        }
        body_with_error = body_with_error.child(body);

        div()
            .key_context("SettingsView")
            .track_focus(&self.panel_focus_handle)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "tab" => {
                        if ev.keystroke.modifiers.shift {
                            window.focus_prev(cx);
                        } else {
                            window.focus_next(cx);
                        }
                        cx.stop_propagation();
                    }
                    "escape" => {
                        this.dismiss(window, cx);
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .size_full()
            .tab_group()
            .flex()
            .flex_col()
            .bg(panel_bg)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .relative()
                    .overflow_hidden()
                    .child(sidebar)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .relative()
                            .overflow_hidden()
                            .child(
                                div()
                                    .id("settings-scroll")
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .right_0()
                                    .bottom_0()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll_handle)
                                    .p(px(theme::SETTINGS_CONTENT_PAD))
                                    .child(body_with_error),
                            )
                            .child(settings_scrollbar(
                                "settings-scrollbar",
                                &self.scroll_handle,
                            )),
                    ),
            )
    }
}

impl SettingsView {
    /// Build the active page's body. New sections are wired here.
    fn render_section_body(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(page) = self.render_layout_page(self.active_section, cx) {
            return page;
        }
        match self.active_section {
            BuiltinSection::Keymap => self.render_keymap(cx),
            BuiltinSection::SessionHosts => self.render_session_hosts(cx),
            BuiltinSection::Accounts => self.render_accounts(cx),
            BuiltinSection::Plugin => self.render_plugin(cx),
            // Every other page has a layout and returned above.
            _ => div().into_any_element(),
        }
    }

    /// While a query is typed the body lists what it matched, grouped by page
    /// and card, each row the live control it is on its own page.
    fn render_search_results(
        &self,
        query: &str,
        results: &[super::search::Doc],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use super::presentation::{card, page_stack};
        use super::search::Target;

        if results.is_empty() {
            return div()
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .text_color(theme::current(cx).text_muted)
                .child(s::settings_search_none(query))
                .into_any_element();
        }
        let mut body = page_stack();
        let mut index = 0;
        while index < results.len() {
            let first = &results[index];
            let title = if first.card.is_empty() {
                navigation::label(first.section)
            } else {
                s::settings_search_group(&navigation::label(first.section), &first.card)
            };
            let mut group = card(title, cx);
            while index < results.len()
                && results[index].section == first.section
                && results[index].card == first.card
            {
                let doc = &results[index];
                let row = match doc.target {
                    Target::Page(section) => self.link_row(
                        gpui::ElementId::Name(format!("settings-search-link-{index}").into()),
                        doc.label.clone(),
                        doc.hint.clone(),
                        s::settings_search_open(),
                        section,
                        cx,
                    ),
                    target => self.render_target_row(target, cx),
                };
                group = group.child(row);
                index += 1;
            }
            body = body.child(group);
        }
        body.into_any_element()
    }

    fn render_sidebar_nav(
        &self,
        query: &str,
        results: &[super::search::Doc],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.active_section;
        let counts = super::search::counts(results);
        let mut list = div()
            .flex()
            .flex_col()
            .py(px(theme::SETTINGS_SIDEBAR_PAD_Y));
        for (sections, group_label) in navigation::GROUPS {
            let matching = sections
                .iter()
                .copied()
                .filter_map(|section| {
                    let count = counts.iter().find(|(s, _)| *s == section).map(|(_, n)| *n);
                    (query.is_empty() || count.is_some())
                        .then(|| (section, navigation::label(section), count))
                })
                .collect::<Vec<_>>();
            if matching.is_empty() {
                continue;
            }
            list = list.child(
                div()
                    .px(px(theme::SETTINGS_SIDEBAR_ROW_PAD_X))
                    .pt(px(theme::MODAL_PANEL_GAP))
                    .pb(px(theme::MODAL_FOOTER_GAP))
                    .text_size(px(theme::TAB_FONT_SIZE))
                    .text_color(theme::current(cx).text_muted)
                    .child(group_label()),
            );
            for (section, label, count) in matching {
                let is_active = query.is_empty() && section == active;
                list = list.child(self.render_sidebar_row(cx, section, label, count, is_active));
            }
        }

        let sidebar_bg = theme::current(cx).settings_sidebar_bg;
        div()
            .flex_none()
            .w(px(theme::SETTINGS_SIDEBAR_W))
            .h_full()
            .bg(sidebar_bg)
            .border_r_1()
            .border_color(theme::current(cx).border)
            .flex()
            .flex_col()
            // Back sits above the search field rather than in the title bar:
            // the host window owns that bar, and teaching it a settings mode
            // would put a mode into chrome that today only knows its tier.
            .child(
                div()
                    .flex_none()
                    .px(px(theme::SETTINGS_SIDEBAR_ROW_PAD_X))
                    .pt(px(theme::SETTINGS_SIDEBAR_PAD_Y))
                    .flex()
                    .flex_col()
                    .gap(px(theme::MODAL_PANEL_GAP))
                    .pb(px(theme::MODAL_PANEL_GAP))
                    .child(
                        crate::ui::button_with_icon(
                            "settings-back",
                            s::settings_back(),
                            crate::ui::icons::BACK,
                        )
                        .tab_stop(true)
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                this.dismiss(window, cx);
                            },
                        )),
                    )
                    .child(div().w_full().child(crate::ui::input(
                        &self.sidebar_search_input,
                        cx,
                        0,
                    ))),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("settings-sidebar-scroll")
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .pr(px(theme::SCROLL_AREA_GUTTER))
                            .overflow_y_scroll()
                            .track_scroll(&self.sidebar_scroll_handle)
                            .child(list),
                    )
                    .child(settings_scrollbar(
                        "settings-sidebar-scrollbar",
                        &self.sidebar_scroll_handle,
                    )),
            )
            // Every page reaches the file, so a setting the UI does not
            // cover yet is one click from any of them.
            .child(
                div()
                    .flex_none()
                    .px(px(theme::SETTINGS_SIDEBAR_ROW_PAD_X))
                    .py(px(theme::MODAL_PANEL_GAP))
                    .border_t_1()
                    .border_color(theme::current(cx).border)
                    .child(
                        crate::ui::button_with_icon(
                            "settings-sidebar-open-config",
                            s::settings_open_config_file(),
                            crate::ui::icons::EDIT,
                        )
                        .tab_stop(true)
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _window, cx| this.open_config_file(cx),
                        )),
                    ),
            )
            .into_any_element()
    }

    fn render_sidebar_row(
        &self,
        cx: &mut Context<Self>,
        section: BuiltinSection,
        label: impl Into<gpui::SharedString>,
        count: Option<usize>,
        is_active: bool,
    ) -> impl IntoElement {
        let row_text = theme::current(cx).text_primary;
        let active_bg = theme::current(cx).overlay_prominent;
        let hover_bg = theme::current(cx).overlay_selected;
        let focus = self
            .sidebar_focus_handles
            .get(&section)
            .expect("every settings section has a sidebar focus handle")
            .clone();

        let row_id: gpui::ElementId =
            gpui::ElementId::Name(format!("settings-nav-{}", section.slug()).into());
        let mut row = div()
            .id(row_id)
            .track_focus(&focus)
            .flex()
            .flex_row()
            .items_center()
            .px(px(theme::SETTINGS_SIDEBAR_ROW_PAD_X))
            .py(px(theme::SETTINGS_SIDEBAR_ROW_PAD_Y))
            .gap(px(theme::PAD_STANDARD))
            .border_l(px(theme::SETTINGS_ACTIVE_BORDER))
            .border_color(if is_active {
                theme::ACCENT
            } else {
                theme::with_alpha(row_text, 0.)
            })
            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
            .text_color(row_text)
            .cursor_pointer()
            .focus_visible(|style| style.border_color(theme::ACCENT))
            .child(crate::ui::icons::icon(navigation::icon(section)))
            .child(label.into())
            .children(count.map(|n| {
                div()
                    .ml_auto()
                    .px(px(theme::PAD_SM))
                    .rounded_full()
                    .bg(theme::current(cx).overlay_selected)
                    .text_size(px(theme::TAB_FONT_SIZE))
                    .text_color(theme::current(cx).text_muted)
                    .child(n.to_string())
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.open_section(section, window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.open_section(section, window, cx);
            }));
        if is_active {
            row = row.bg(active_bg);
        } else {
            row = row.hover(move |el| el.bg(hover_bg));
        }
        row
    }
}

fn settings_scrollbar(id: &'static str, scroll_handle: &gpui::ScrollHandle) -> impl IntoElement {
    // Prepaint reads the current content bounds, including page switches and
    // resize. The inset layer pins the track over its own scroll viewport.
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .child(
            crate::ui::scrollbar::Scrollbar::vertical(scroll_handle)
                .id(id)
                .scrollbar_show(crate::ui::scrollbar::ScrollbarShow::Always),
        )
}
