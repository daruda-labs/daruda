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

mod about;
mod plugin;

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{button, button_danger, checkbox, checkbox_row, field_row};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;
use gpui::{AnyElement, ClickEvent, ClipboardItem, IntoElement, div, prelude::*, px};

use super::SettingsWindow;

/// How long the pairing-command "Copied!" label stays before reverting
/// to "Copy" — mirrors `ErrorReportModal::COPIED_LABEL_DURATION`
/// (`workspace/error/modal.rs`), duplicated locally since it's a
/// trivial, single-use UI-timing constant not worth centralizing.
const TELEGRAM_PAIR_COPY_LABEL_DURATION: std::time::Duration = std::time::Duration::from_secs(1);

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
    /// Copy `/pair <code>` to the clipboard so the user can paste it
    /// straight into the Telegram app on their phone instead of retyping
    /// it. Mirrors `ErrorReportModal::copy_to_clipboard`'s copied/revert
    /// shape (`workspace/error/modal.rs`).
    fn copy_telegram_pair_command(&mut self, code: &str, cx: &mut gpui::Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(format!("/pair {code}")));
        self.telegram_pair_command_copied = true;
        cx.notify();

        self._telegram_pair_copy_revert_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(TELEGRAM_PAIR_COPY_LABEL_DURATION)
                .await;
            // SILENT-OK: settings window may close before the revert timer fires
            let _ = this.update(cx, |this, cx| {
                if this.telegram_pair_command_copied {
                    this.telegram_pair_command_copied = false;
                    cx.notify();
                }
            });
        }));
    }

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
                s::settings_label_editor_font_size(),
                crate::ui::input(&self.editor_font_size_input, cx, ()),
            ))
            .child(field_row(
                s::settings_label_agent_chat_font_size(),
                crate::ui::input(&self.agent_chat_font_size_input, cx, ()),
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

    pub(super) fn render_agent(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        use daruda_config::DefaultPermissionMode as M;
        let description_color = theme::current(cx).text_muted;
        let use_modifier_to_send = self.agent_use_modifier_to_send;
        // The dropdown shows the bare mode id; this blurb explains the
        // currently selected mode and updates on each pick (the window
        // re-renders via the select's Confirm subscription).
        let selected_mode = self
            .default_permission_mode_select
            .read(cx)
            .selected_value()
            .and_then(|v| M::from_mode_id(v.as_ref()))
            .unwrap_or_default();
        let mode_description = match selected_mode {
            M::Default => s::settings_agent_mode_default(),
            M::AcceptEdits => s::settings_agent_mode_accept_edits(),
            M::Plan => s::settings_agent_mode_plan(),
            M::BypassPermissions => s::settings_agent_mode_bypass(),
        };
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_agent(), cx))
            .child(field_row(
                s::settings_label_agent_mode(),
                crate::ui::select::select(&self.default_permission_mode_select, cx, ()),
            ))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(description_color)
                    .child(mode_description),
            )
            .child(checkbox_row(
                checkbox(
                    "settings-agent-use-modifier-to-send",
                    s::settings_label_agent_use_modifier_to_send(),
                    (),
                )
                .checked(use_modifier_to_send)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.agent_use_modifier_to_send = *checked;
                    cx.notify();
                })),
            ))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(description_color)
                    .child(s::settings_agent_use_modifier_to_send_description()),
            );

        body = body
            .child(Self::section_label(s::settings_section_agent_catalog(), cx))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(description_color)
                    .child(s::settings_agent_catalog_description()),
            )
            .child(field_row(
                s::settings_agent_preset(),
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .child(div().flex_1().child(crate::ui::select::select(
                        &self.agent_preset_select,
                        cx,
                        (),
                    )))
                    .child(
                        button("settings-agent-add-preset", s::settings_agent_add_preset())
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                let selected = this
                                    .agent_preset_select
                                    .read(cx)
                                    .selected_value()
                                    .map(|id| id.to_string());
                                if let Some(definition) = selected.and_then(|id| {
                                    daruda_config::AgentDefinition::registry_preset(&id)
                                }) {
                                    this.add_agent_row(definition, window, cx);
                                }
                            })),
                    )
                    .child(
                        button("settings-agent-add-custom", s::settings_agent_add_custom())
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.add_agent_row(
                                    daruda_config::AgentDefinition {
                                        id: String::new(),
                                        name: String::new(),
                                        launch: daruda_config::AgentLaunch::Raw(String::new()),
                                    },
                                    window,
                                    cx,
                                );
                            })),
                    ),
            ));

        if self.agent_rows.is_empty() {
            body = body.child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(description_color)
                    .child(s::settings_agent_catalog_empty()),
            );
        }
        for (index, row) in self.agent_rows.iter().enumerate() {
            body = body.child(self.render_agent_catalog_row(index, row, cx));
        }

        body.into_any_element()
    }

    fn render_agent_catalog_row(
        &self,
        index: usize,
        row: &super::AgentCatalogRow,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let t = theme::current(cx);
        let remove_id = format!("settings-agent-remove-{index}");
        let transport_kind = row
            .transport_select
            .read(cx)
            .selected_value()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "raw".to_string());

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .p(px(theme::MODAL_PANEL_GAP))
            .border_1()
            .border_color(t.border)
            .rounded(px(theme::RADIUS_MD))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                            .text_color(t.text_primary)
                            .child(s::settings_agent_catalog_row_label(index + 1)),
                    )
                    .child(
                        button_danger(remove_id, s::settings_agent_remove()).on_click(cx.listener(
                            move |this, _: &ClickEvent, _window, cx| {
                                this.remove_agent_row(index, cx);
                            },
                        )),
                    ),
            )
            .child(field_row(
                s::settings_agent_field_id(),
                crate::ui::input(&row.id_input, cx, ()),
            ))
            .child(field_row(
                s::settings_agent_field_name(),
                crate::ui::input(&row.name_input, cx, ()),
            ))
            .child(field_row(
                s::settings_agent_field_command(),
                crate::ui::input(&row.command_input, cx, ()),
            ))
            .child(field_row(
                s::settings_agent_field_transport(),
                crate::ui::select::select(&row.transport_select, cx, ()),
            ));

        // Only one of host/container is meaningful per transport kind — show
        // just that field, plus a hint pointing at the Lane's remote-path
        // setting (the value `AgentLaunch::wrap` substitutes in at connect
        // time; see `daruda_config::AgentLaunch::needs_remote_cwd`).
        if transport_kind == "ssh" {
            body = body
                .child(field_row(
                    s::settings_agent_field_host(),
                    crate::ui::input(&row.host_input, cx, ()),
                ))
                .child(
                    div()
                        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                        .text_color(t.text_muted)
                        .child(s::settings_agent_remote_path_hint()),
                );
        } else if transport_kind == "docker" {
            body = body
                .child(field_row(
                    s::settings_agent_field_container(),
                    crate::ui::input(&row.container_input, cx, ()),
                ))
                .child(
                    div()
                        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                        .text_color(t.text_muted)
                        .child(s::settings_agent_remote_path_hint()),
                );
        }

        body.into_any_element()
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
        let body_color = theme::current(cx).text_primary;
        let telegram_enabled = self.telegram_enabled;
        let token_configured = self.telegram_token_configured;
        let authorized_chat_id = crate::settings_store::SettingsStore::global(cx)
            .user_arc()
            .telegram
            .authorized_chat_id;
        let pair_code = self.telegram_pair_code.clone();

        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_notifications(), cx))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(body_color)
                    .child(s::settings_telegram_heading()),
            )
            .child(checkbox_row(
                checkbox(
                    "settings-telegram-enabled",
                    s::settings_telegram_enabled_label(),
                    (),
                )
                .checked(telegram_enabled)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.telegram_enabled = *checked;
                    cx.notify();
                })),
            ))
            .child(field_row(
                s::settings_telegram_token_label(),
                crate::ui::input(&self.telegram_token_input, cx, ()),
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .child(
                        button(
                            "settings-telegram-save-token",
                            s::settings_telegram_save_token(),
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                let token = this
                                    .telegram_token_input
                                    .read(cx)
                                    .value()
                                    .trim()
                                    .to_string();
                                if token.is_empty() {
                                    return;
                                }
                                match crate::telegram::keychain::write_token(&token) {
                                    Ok(()) => {
                                        this.telegram_token_configured = true;
                                        this.telegram_token_input.update(cx, |input, cx_state| {
                                            input.set_value(String::new(), window, cx_state);
                                        });
                                    }
                                    Err(_) => {
                                        // `keychain::write_token` already logs the
                                        // failure via `LogWriter::log` — nothing left
                                        // to do here beyond the UI state below.
                                    }
                                }
                                cx.notify();
                            },
                        )),
                    )
                    .when(token_configured, |row| {
                        row.child(
                            div()
                                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                                .text_color(body_color)
                                .child(s::settings_telegram_token_configured()),
                        )
                        .child(
                            button_danger(
                                "settings-telegram-clear-token",
                                s::settings_telegram_clear_token(),
                            )
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, _window, cx| {
                                    // `keychain::delete_token` already logs any
                                    // failure via `LogWriter::log`. Only flip the
                                    // "configured" state on success — but still
                                    // notify unconditionally, matching the other
                                    // Telegram button handlers.
                                    if crate::telegram::keychain::delete_token().is_ok() {
                                        this.telegram_token_configured = false;
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .child(
                        div()
                            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                            .text_color(body_color)
                            .child(match authorized_chat_id {
                                Some(chat_id) => s::settings_telegram_paired(chat_id),
                                None => s::settings_telegram_not_paired(),
                            }),
                    )
                    .child(
                        // The bridge's poll loop pairs a phone in the background
                        // (via `/pair <code>`) — this section only re-reads
                        // `authorized_chat_id` when IT re-renders, and nothing
                        // currently subscribes this window to that background
                        // change. This button forces a repaint with no state
                        // mutation of its own, so the status line above picks up
                        // whatever `SettingsStore`'s live config already has.
                        button(
                            "settings-telegram-check-pairing",
                            s::settings_telegram_check_pairing(),
                        )
                        .on_click(cx.listener(
                            |_this, _: &ClickEvent, _window, cx| {
                                cx.notify();
                            },
                        )),
                    ),
            )
            .when_some(pair_code, |body, code| {
                let copy_label = if self.telegram_pair_command_copied {
                    s::error_modal_button_copied()
                } else {
                    s::error_modal_button_copy()
                };
                body.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(theme::MODAL_FOOTER_GAP))
                        .child(
                            div()
                                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                                .text_color(body_color)
                                .child(s::settings_telegram_pair_instructions(&code)),
                        )
                        .child(
                            button("settings-telegram-copy-pair-command", copy_label).on_click(
                                cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                    this.copy_telegram_pair_command(&code, cx);
                                }),
                            ),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .child(
                        button(
                            "settings-telegram-generate-code",
                            s::settings_telegram_generate_code(),
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _window, cx| {
                                let code =
                                    crate::telegram::global::TelegramBridge::generate_pair_code(cx);
                                this.telegram_pair_code = Some(code);
                                cx.notify();
                            },
                        )),
                    )
                    .when(authorized_chat_id.is_some(), |row| {
                        row.child(
                            button_danger(
                                "settings-telegram-unpair",
                                s::settings_telegram_unpair(),
                            )
                            .on_click(cx.listener(
                                |_this, _: &ClickEvent, _window, cx| {
                                    use gpui::BorrowAppContext as _;
                                    let result = cx
                                        .update_global::<crate::settings_store::SettingsStore, _>(
                                            |store, _| {
                                                store.patch_user(|cfg| {
                                                    cfg.telegram.authorized_chat_id = None
                                                })
                                            },
                                        );
                                    if let Err(e) = result {
                                        LogWriter::log(
                                            ErrorReport::new("Failed to clear Telegram pairing")
                                                .severity(ErrorSeverity::Warning)
                                                .message(e)
                                                .at(file!(), line!())
                                                .dedup("telegram.unpair")
                                                .build(),
                                        );
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                    }),
            )
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(body_color)
                    .child(s::settings_placeholder_notifications()),
            )
            .child(Self::render_open_config_button(cx))
            .into_any_element()
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
        let body_color = theme::current(cx).text_primary;
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
            .child(Self::render_open_config_button(cx))
            .into_any_element()
    }

    /// "Open Config File" button — creates the config directory if
    /// missing, then opens `config.toml` in the user's default editor
    /// for the file type. Shared by [`render_placeholder`] (sections
    /// with no GUI yet) and [`render_notifications`] (the Telegram
    /// block's "everything else" fallback).
    fn render_open_config_button(cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().flex().flex_row().child(
            button("settings-open-config", s::settings_open_config_file()).on_click(cx.listener(
                |_this, _: &ClickEvent, _window, cx| {
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
                },
            )),
        )
    }
}

#[cfg(test)]
#[allow(dead_code)] // exposed for tests that exercise the section without rendering it.
impl SettingsWindow {
    pub(in crate::settings_window) fn telegram_pair_command_copied(&self) -> bool {
        self.telegram_pair_command_copied
    }

    /// Test-only entry into [`Self::copy_telegram_pair_command`] — the
    /// click handler that drives it lives inside a closure and isn't
    /// directly callable from tests. Mirrors
    /// `ErrorReportModal::copy_to_clipboard_for_test`.
    pub(in crate::settings_window) fn copy_telegram_pair_command_for_test(
        &mut self,
        code: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        self.copy_telegram_pair_command(code, cx);
    }
}
