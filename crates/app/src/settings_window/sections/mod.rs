//! Per-section body renderers for `SettingsWindow`.
//!
//! Each method here builds the form for one [`BuiltinSection`] page.
//! `render::render_section_body` matches on the active section and
//! dispatches to the appropriate method. Adding a new section:
//! method here + match arm in `render::render_section_body` +
//! sidebar nav row.
//!
//! The Plugin section lives in the [`plugin`] submodule; its
//! `impl Workspace` block extends the same type through the
//! standard sibling-module pattern.

mod plugin;

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{button, checkbox, checkbox_row, field_row};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;
use gpui::{AnyElement, ClickEvent, IntoElement, div, prelude::*, px};

use super::SettingsWindow;

/// Build a `file://` URL from `path`, percent-encoding any byte that is
/// not safe in a URL path segment (RFC 3986 §3.3). Handles spaces in
/// paths like `~/Library/Application Support/…`.
fn path_to_file_url(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    let mut encoded = String::with_capacity(bytes.len() + 8);
    for &b in bytes {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b'/'
            | b':'
            | b'@'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'=' => {
                encoded.push(b as char);
            }
            _ => {
                encoded.push('%');
                let hi = b >> 4;
                let lo = b & 0xF;
                encoded.push(if hi < 10 {
                    (b'0' + hi) as char
                } else {
                    (b'A' + hi - 10) as char
                });
                encoded.push(if lo < 10 {
                    (b'0' + lo) as char
                } else {
                    (b'A' + lo - 10) as char
                });
            }
        }
    }
    format!("file://{encoded}")
}

impl SettingsWindow {
    pub(super) fn render_general(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        // Phase 3-D ships two UI presets (`daruda_dark` / `daruda_light`)
        // so the dropdown becomes interactive. It still renders as
        // disabled if a future build ships only one preset, so the
        // row's layout is stable across that edge case.
        let ui_preset_disabled = daruda_config::UI_THEME_PRESETS.len() <= 1;
        let ui_preset_widget = crate::ui::select::select(&self.ui_preset_select, cx, ())
            .when(ui_preset_disabled, |w| w.disabled(true));

        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_general(), cx))
            .child(field_row(
                s::settings_label_language(),
                crate::ui::select::select(&self.language_select, cx, ()),
            ))
            .child(field_row(
                s::settings_label_terminal_theme(),
                crate::ui::select::select(&self.terminal_preset_select, cx, ()),
            ))
            .child(field_row(s::settings_label_ui_theme(), ui_preset_widget))
            .child(field_row(
                s::settings_label_syntax_theme(),
                crate::ui::select::select(&self.syntax_theme_select, cx, ()),
            ))
            .into_any_element()
    }

    pub(super) fn render_font(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_font(), cx))
            .child(field_row(
                s::settings_label_font_family(),
                crate::ui::select::select(&self.font_family_select, cx, ()),
            ))
            .child(field_row(
                s::settings_label_font_size(),
                crate::ui::input(&self.font_size_input, cx, ()),
            ))
            .child(field_row(
                s::settings_label_vertical_spacing(),
                crate::ui::input(&self.vertical_spacing_input, cx, ()),
            ))
            .child(field_row(
                s::settings_label_horizontal_spacing(),
                crate::ui::input(&self.horizontal_spacing_input, cx, ()),
            ))
            .into_any_element()
    }

    pub(super) fn render_cursor(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let cursor_blinking = self.cursor_blinking;
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_cursor(), cx))
            .child(field_row(
                s::settings_label_cursor_style(),
                crate::ui::select::select(&self.cursor_style_select, cx, ()),
            ))
            .child(checkbox_row(
                checkbox(
                    "settings-cursor-blinking",
                    s::settings_label_cursor_blinking(),
                    (),
                )
                .checked(cursor_blinking)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.cursor_blinking = *checked;
                    cx.notify();
                })),
            ))
            .into_any_element()
    }

    pub(super) fn render_shell(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let close_on_exit = self.close_pane_on_exit;
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_shell(), cx))
            .child(checkbox_row(
                checkbox(
                    "settings-close-on-exit",
                    s::settings_label_close_on_exit(),
                    (),
                )
                .checked(close_on_exit)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.close_pane_on_exit = *checked;
                    cx.notify();
                })),
            ))
            .into_any_element()
    }

    pub(super) fn render_window(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let window_blur = self.window_blur;
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_window(), cx))
            .child(field_row(
                s::settings_label_window_opacity(),
                crate::ui::input(&self.opacity_input, cx, ()),
            ))
            .child(checkbox_row(
                checkbox("settings-window-blur", s::settings_label_window_blur(), ())
                    .checked(window_blur)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.window_blur = *checked;
                        cx.notify();
                    })),
            ))
            .into_any_element()
    }

    pub(super) fn render_terminal(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_terminal(), cx))
            .child(field_row(
                s::settings_label_scrollback(),
                crate::ui::input(&self.scrollback_input, cx, ()),
            ))
            .child(field_row(
                s::settings_label_max_fps(),
                crate::ui::select::select(&self.max_fps_select, cx, ()),
            ))
            .child(field_row(
                s::settings_label_inset_x(),
                crate::ui::input(&self.inset_x_input, cx, ()),
            ))
            .child(field_row(
                s::settings_label_inset_y(),
                crate::ui::input(&self.inset_y_input, cx, ()),
            ))
            .into_any_element()
    }

    pub(super) fn render_sidebar(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let files_show_hidden = self.files_show_hidden;
        let files_use_gitignore = self.files_use_gitignore;
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_sidebar(), cx))
            .child(checkbox_row(
                checkbox("settings-show-hidden", s::settings_label_show_hidden(), ())
                    .checked(files_show_hidden)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.files_show_hidden = *checked;
                        cx.notify();
                    })),
            ))
            .child(checkbox_row(
                checkbox(
                    "settings-use-gitignore",
                    s::settings_label_use_gitignore(),
                    (),
                )
                .checked(files_use_gitignore)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.files_use_gitignore = *checked;
                    cx.notify();
                })),
            ))
            .into_any_element()
    }

    pub(super) fn render_clipboard(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_clipboard(), cx))
            .child(field_row(
                s::settings_label_clipboard_streaming(),
                crate::ui::input(&self.clipboard_streaming_input, cx, ()),
            ))
            .into_any_element()
    }

    pub(super) fn render_panels(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_panels(), cx))
            .child(field_row(
                s::settings_label_grid_columns(),
                crate::ui::input(&self.panels_grid_columns_input, cx, ()),
            ))
            .into_any_element()
    }

    pub(super) fn render_claude_status(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let claude_status_enable = self.claude_status_enable;
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_claude_status(), cx))
            .child(checkbox_row(
                checkbox(
                    "settings-claude-status-enable",
                    s::settings_label_claude_status_enable(),
                    (),
                )
                .checked(claude_status_enable)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.claude_status_enable = *checked;
                    cx.notify();
                })),
            ))
            .into_any_element()
    }

    pub(super) fn render_notifications(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        Self::render_placeholder(
            s::settings_section_notifications(),
            s::settings_placeholder_notifications(),
            cx,
        )
    }

    pub(super) fn render_keymap(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        Self::render_placeholder(
            s::settings_section_keymap(),
            s::settings_placeholder_keymap(),
            cx,
        )
    }

    /// Shared body for sections that have no GUI yet — shows the
    /// section header + an explanation pointing the user at the
    /// config file, plus a button to open it directly.
    fn render_placeholder(
        header: impl Into<gpui::SharedString>,
        body: impl Into<gpui::SharedString>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let body_color = theme::TEXT_PRIMARY;
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(header, cx))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(body_color)
                    .child(body.into()),
            )
            .child(div().flex().flex_row().child(
                button("settings-open-config", s::settings_open_config_file()).on_click(
                    cx.listener(|_this, _: &ClickEvent, _window, cx| {
                        let path = daruda_config::config_path();
                        if let Some(parent) = path.parent()
                            && let Err(e) = std::fs::create_dir_all(parent)
                        {
                            LogWriter::log(
                                ErrorReport::new("Failed to create config directory")
                                    .severity(ErrorSeverity::Warning)
                                    .from_error(&e)
                                    .at(file!(), line!())
                                    .with_context("path", redact_home(parent))
                                    .dedup("config.mkdir")
                                    .build(),
                            );
                        }
                        cx.open_url(&path_to_file_url(&path));
                    }),
                ),
            ))
            .into_any_element()
    }
}
