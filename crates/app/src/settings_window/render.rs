//! `impl Render for SettingsWindow` — sidebar + body layout.

use crate::ui::theme;
use daruda_config::BuiltinSection;
use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, KeyDownEvent, Render, Window, div, prelude::*, px,
};

use super::{SettingsWindow, settings_button as button};
use crate::surface::strings as s;

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        // Match the workspace title bar's themed token (retones for light mode)
        // rather than the fixed-dark `SURFACE_1` const, which left dark header
        // text on a dark bar in light mode.
        let header_bg = t.title_bar_bg;
        let header_text = t.text_primary;
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

        let mut body_with_error = div().flex().flex_col();
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
        } else if let Some(err) = self.error.as_ref() {
            body_with_error = body_with_error.child(
                div()
                    .pb(px(theme::MODAL_PANEL_GAP))
                    .child(crate::ui::alert::error("settings-error", err.clone())),
            );
        }
        body_with_error = body_with_error.child(body);

        div()
            .key_context("SettingsWindow")
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
                        this.dismiss(window);
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
            BuiltinSection::ExternalEditor => self.render_editor(cx),
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
        let query = self
            .sidebar_search_input
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let mut list = div()
            .flex()
            .flex_col()
            .py(px(theme::SETTINGS_SIDEBAR_PAD_Y));
        for group in SidebarGroup::ALL {
            let matching = BuiltinSection::ALL
                .iter()
                .copied()
                .filter(|section| sidebar_group(*section) == group)
                .filter_map(|section| {
                    let label = section_nav_label(section);
                    let matches = query.is_empty()
                        || label.to_lowercase().contains(&query)
                        || section.slug().contains(&query);
                    matches.then_some((section, label))
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
                    .child(sidebar_group_label(group)),
            );
            for (section, label) in matching {
                let is_active = section == active;
                list = list.child(self.render_sidebar_row(cx, section, label, is_active));
            }
        }

        let sidebar_bg = theme::current(cx).settings_sidebar_bg;
        div()
            .flex_none()
            .w(px(theme::SETTINGS_SIDEBAR_W))
            .h_full()
            .bg(sidebar_bg)
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(px(theme::SETTINGS_SIDEBAR_ROW_PAD_X))
                    .pt(px(theme::SETTINGS_SIDEBAR_PAD_Y))
                    .child(crate::ui::input(&self.sidebar_search_input, cx, 0)),
            )
            .child(
                div()
                    .id("settings-sidebar-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(list),
            )
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
            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
            .text_color(row_text)
            .cursor_pointer()
            .child(label.into())
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.focus_section(section, window, cx);
                    cx.stop_propagation();
                }
            }))
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
        BuiltinSection::ExternalEditor => s::settings_nav_external_editor(),
        BuiltinSection::Agent => s::settings_nav_agent(),
        BuiltinSection::SessionHosts => s::settings_nav_session_hosts(),
        BuiltinSection::Accounts => s::settings_nav_accounts(),
        BuiltinSection::Notifications => s::settings_nav_notifications(),
        BuiltinSection::Keymap => s::settings_nav_keymap(),
        BuiltinSection::Plugin => s::settings_nav_plugin(),
        BuiltinSection::About => s::settings_nav_about(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarGroup {
    General,
    Appearance,
    Terminal,
    Workspace,
    Agents,
    Integrations,
    System,
}

impl SidebarGroup {
    const ALL: [Self; 7] = [
        Self::General,
        Self::Appearance,
        Self::Terminal,
        Self::Workspace,
        Self::Agents,
        Self::Integrations,
        Self::System,
    ];
}

fn sidebar_group(section: BuiltinSection) -> SidebarGroup {
    match section {
        BuiltinSection::General => SidebarGroup::General,
        BuiltinSection::Font | BuiltinSection::Cursor | BuiltinSection::Window => {
            SidebarGroup::Appearance
        }
        BuiltinSection::Shell | BuiltinSection::Terminal => SidebarGroup::Terminal,
        BuiltinSection::Dock | BuiltinSection::Clipboard | BuiltinSection::ExternalEditor => {
            SidebarGroup::Workspace
        }
        BuiltinSection::Agent | BuiltinSection::SessionHosts | BuiltinSection::Accounts => {
            SidebarGroup::Agents
        }
        BuiltinSection::Notifications | BuiltinSection::Plugin => SidebarGroup::Integrations,
        BuiltinSection::Keymap | BuiltinSection::About => SidebarGroup::System,
    }
}

fn sidebar_group_label(group: SidebarGroup) -> String {
    match group {
        SidebarGroup::General => s::settings_group_general(),
        SidebarGroup::Appearance => s::settings_group_appearance(),
        SidebarGroup::Terminal => s::settings_group_terminal(),
        SidebarGroup::Workspace => s::settings_group_workspace(),
        SidebarGroup::Agents => s::settings_group_agents(),
        SidebarGroup::Integrations => s::settings_group_integrations(),
        SidebarGroup::System => s::settings_group_system(),
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
