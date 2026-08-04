//! `impl Render for SettingsWindow` — sidebar + body layout.

use crate::ui::theme;
use daruda_config::BuiltinSection;
use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, KeyDownEvent, Render, Window, div, prelude::*, px,
};

use crate::ui::{button, button_primary};

use super::SettingsWindow;
use crate::surface::strings as s;

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        // Match the workspace title bar's themed token (retones for light mode)
        // rather than the fixed-dark `SURFACE_1` const, which left dark header
        // text on a dark bar in light mode.
        let header_bg = t.title_bar_bg;
        let header_text = t.text_primary;
        let error_text = theme::ERROR;
        let panel_bg = t.modal_panel_bg;

        // Header bar (window-wide, above sidebar+body).
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(theme::TAB_BAR_HEIGHT))
            .flex_none()
            .bg(header_bg)
            .pl(px(theme::TRAFFIC_LIGHT_WIDTH))
            .text_size(px(theme::TAB_FONT_SIZE))
            .text_color(header_text)
            .child(s::settings_title());

        let body = self.render_section_body(cx);
        let sidebar = self.render_sidebar_nav(cx);

        let mut body_with_error = div().flex().flex_col().child(body);
        if let Some(err) = self.error.as_ref() {
            body_with_error = body_with_error.child(
                div()
                    .pt(px(theme::MODAL_PANEL_GAP))
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(error_text)
                    .child(err.clone()),
            );
        }

        // Footer bar.
        let footer_bar = div()
            .flex_none()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .px(px(theme::MODAL_PANEL_PAD))
            .py(px(theme::MODAL_FOOTER_GAP))
            .bg(panel_bg)
            .child(
                button("settings-cancel", s::settings_cancel()).on_click(cx.listener(
                    |this, _: &ClickEvent, window, _cx| {
                        this.dismiss(window);
                    },
                )),
            )
            .child(
                button_primary("settings-save", s::settings_save()).on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        this.submit(window, cx);
                    },
                )),
            );

        div()
            .key_context("SettingsWindow")
            .track_focus(&self.panel_focus_handle)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "tab" => {
                        this.focus_next_input(!ev.keystroke.modifiers.shift, window, cx);
                        cx.stop_propagation();
                    }
                    "escape" => {
                        this.dismiss(window);
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(panel_bg)
            .child(header)
            .child(
                // Sidebar + body row fills space between header and footer.
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
                                    .px(px(theme::MODAL_PANEL_PAD))
                                    .pt(px(theme::MODAL_PANEL_PAD))
                                    .pb(px(theme::MODAL_FOOTER_GAP))
                                    .child(body_with_error),
                            )
                            .children(settings_scrollbar(&self.scroll_handle, cx)),
                    ),
            )
            .child(footer_bar)
    }
}

impl SettingsWindow {
    /// Build the active page's body. New sections are wired here.
    fn render_section_body(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.active_section {
            BuiltinSection::General => self.render_general(cx),
            BuiltinSection::Font => self.render_font(cx),
            BuiltinSection::Cursor => self.render_cursor(cx),
            BuiltinSection::Shell => self.render_shell(cx),
            BuiltinSection::Window => self.render_window(cx),
            BuiltinSection::Terminal => self.render_terminal(cx),
            BuiltinSection::Dock => self.render_dock(cx),
            BuiltinSection::Clipboard => self.render_clipboard(cx),
            BuiltinSection::Agent => self.render_agent(cx),
            BuiltinSection::SessionHosts => self.render_session_hosts(cx),
            BuiltinSection::Accounts => self.render_accounts(cx),
            BuiltinSection::Notifications => self.render_notifications(cx),
            BuiltinSection::Keymap => self.render_keymap(cx),
            BuiltinSection::Plugin => self.render_plugin(cx),
            BuiltinSection::About => self.render_about(cx),
        }
    }

    fn render_sidebar_nav(&self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.active_section;
        let mut list = div()
            .flex()
            .flex_col()
            .py(px(theme::SETTINGS_SIDEBAR_PAD_Y));
        for &section in BuiltinSection::ALL {
            let label = section_nav_label(section);
            let is_active = section == active;
            list = list.child(self.render_sidebar_row(cx, section, label, is_active));
        }

        let sidebar_bg = theme::current(cx).settings_sidebar_bg;
        div()
            .flex_none()
            .w(px(theme::SETTINGS_SIDEBAR_W))
            .h_full()
            .bg(sidebar_bg)
            .flex()
            .flex_col()
            .child(div().flex_1().overflow_hidden().child(list))
            .into_any_element()
    }

    fn render_sidebar_row(
        &self,
        cx: &mut Context<Self>,
        section: BuiltinSection,
        label: impl Into<gpui::SharedString>,
        is_active: bool,
    ) -> impl IntoElement {
        let row_text = theme::current(cx).text_primary;
        let active_bg = theme::current(cx).overlay_prominent;
        let hover_bg = theme::current(cx).overlay_selected;

        let row_id: gpui::ElementId =
            gpui::ElementId::Name(format!("settings-nav-{}", section.slug()).into());
        let mut row = div()
            .id(row_id)
            .flex()
            .flex_row()
            .items_center()
            .px(px(theme::SETTINGS_SIDEBAR_ROW_PAD_X))
            .py(px(theme::SETTINGS_SIDEBAR_ROW_PAD_Y))
            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
            .text_color(row_text)
            .cursor_pointer()
            .child(label.into())
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.focus_section(section, window, cx);
            }));
        if is_active {
            row = row.bg(active_bg);
        } else {
            row = row.hover(move |el| el.bg(hover_bg));
        }
        row
    }
}

fn section_nav_label(section: BuiltinSection) -> String {
    match section {
        BuiltinSection::General => s::settings_nav_general(),
        BuiltinSection::Font => s::settings_nav_font(),
        BuiltinSection::Cursor => s::settings_nav_cursor(),
        BuiltinSection::Shell => s::settings_nav_shell(),
        BuiltinSection::Window => s::settings_nav_window(),
        BuiltinSection::Terminal => s::settings_nav_terminal(),
        BuiltinSection::Dock => s::settings_nav_dock(),
        BuiltinSection::Clipboard => s::settings_nav_clipboard(),
        BuiltinSection::Agent => s::settings_nav_agent(),
        BuiltinSection::SessionHosts => s::settings_nav_session_hosts(),
        BuiltinSection::Accounts => s::settings_nav_accounts(),
        BuiltinSection::Notifications => s::settings_nav_notifications(),
        BuiltinSection::Keymap => s::settings_nav_keymap(),
        BuiltinSection::Plugin => s::settings_nav_plugin(),
        BuiltinSection::About => s::settings_nav_about(),
    }
}

fn settings_scrollbar(
    scroll_handle: &gpui::ScrollHandle,
    cx: &gpui::App,
) -> Option<crate::ui::scrollbar::Thumb> {
    let viewport_h = scroll_handle.bounds().size.height;
    let max_offset = scroll_handle.max_offset().y;
    let t = theme::current(cx);
    crate::ui::scrollbar::vertical_thumb(
        "settings-scrollbar-thumb",
        viewport_h,
        viewport_h + max_offset,
        scroll_handle.offset().y,
        px(0.),
        t.scrollbar_thumb,
        t.settings_scrollbar_thumb_hover,
    )
}
