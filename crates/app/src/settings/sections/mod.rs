//! Per-section body renderers for `SettingsView`.
//!
//! Each method here builds the form for one [`BuiltinSection`] page.
//! `render::render_section_body` matches on the active section and
//! dispatches to the appropriate method. Adding a new section:
//! method here + match arm in `render::render_section_body` +
//! sidebar nav row.
//!
//! The Agent, Plugin, and Session Hosts sections live in the [`agent`] /
//! [`plugin`] / [`session_hosts`] submodules; their `impl SettingsView`
//! blocks extend the same type through the standard sibling-module pattern.
//! [`agent_vocabulary`] and [`agent_transcript`] are the Agent section's
//! option-sourcing halves — where a catalog row's mode / model pickers and its
//! Fold / Recent steps / Filter pickers get their choices. [`agent_env`] is
//! the pure text ↔ value mapping behind its Environment field.

mod about;
mod accounts;
mod agent;
pub(super) mod agent_env;
pub(super) mod agent_transcript;
pub(super) mod agent_vocabulary;
pub(super) mod orchestrator;
pub(super) mod plugin;
mod session_hosts;

use super::CopyFeedback;
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{checkbox, checkbox_row, field_row};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use gpui::{AnyElement, ClickEvent, ClipboardItem, IntoElement, div, prelude::*, px};

use super::{
    BoolSetting, SettingsView, settings_button as button, settings_button_danger as button_danger,
};

/// How long the pairing-command "Copied!" label stays before reverting
/// to "Copy" — mirrors `ErrorReportModal::COPIED_LABEL_DURATION`
/// (`workspace/error/modal.rs`), duplicated locally since it's a
/// trivial, single-use UI-timing constant not worth centralizing.
const TELEGRAM_PAIR_COPY_LABEL_DURATION: std::time::Duration = std::time::Duration::from_secs(1);

fn font_domain_label(label: impl Into<gpui::SharedString>, cx: &gpui::App) -> impl IntoElement {
    let t = theme::current(cx);
    div()
        .mt(px(theme::PAD_SM))
        .pb(px(theme::PAD_XS))
        .border_b_1()
        .border_color(t.border)
        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(t.text_primary)
        .child(label.into())
}

impl SettingsView {
    /// Put the `/setcommands` block on the clipboard.
    ///
    /// The block's own text stays English in every locale: what it registers
    /// is the command menu Telegram shows, and BotFather parses it as
    /// `name - description` lines whose names must match the commands
    /// `control::spec` actually accepts.
    fn copy_botfather_commands(&mut self, cx: &mut gpui::Context<Self>) {
        self.copy_with_feedback(
            s::control_botfather_commands(),
            |this| &mut this.telegram_botfather_copy,
            cx,
        );
    }

    /// Copy `/pair <code>` to the clipboard so the user can paste it
    /// straight into the Telegram app on their phone instead of retyping
    /// it. Mirrors `ErrorReportModal::copy_to_clipboard`'s copied/revert
    /// shape (`workspace/error/modal.rs`).
    fn copy_telegram_pair_command(&mut self, code: &str, cx: &mut gpui::Context<Self>) {
        self.copy_with_feedback(
            format!("/pair {code}"),
            |this| &mut this.telegram_pair_command_copy,
            cx,
        );
    }

    /// Write `text` to the clipboard and run the Copy → Copied! → Copy label
    /// swap on the [`CopyFeedback`] `slot` names. The single implementation
    /// both copy buttons share; `slot` is a field accessor because the revert
    /// timer has to reach the same field again after awaiting.
    fn copy_with_feedback(
        &mut self,
        text: String,
        slot: fn(&mut Self) -> &mut CopyFeedback,
        cx: &mut gpui::Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        slot(self).copied = true;
        cx.notify();

        let revert = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(TELEGRAM_PAIR_COPY_LABEL_DURATION)
                .await;
            // SILENT-OK: the settings view may go away before the revert timer fires
            let _ = this.update(cx, |this, cx| {
                if slot(this).copied {
                    slot(this).copied = false;
                    cx.notify();
                }
            });
        });
        slot(self)._revert = Some(revert);
    }

    pub(super) fn render_general(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        // Disabled (not hidden) if only one UI preset is available, so the
        // row's layout stays stable across that edge case.
        let ui_preset_disabled = daruda_config::UI_THEME_PRESETS.len() <= 1;
        let ui_preset_widget = crate::ui::select::select(&self.ui_preset_select, cx, 0)
            .when(ui_preset_disabled, |w| w.disabled(true));

        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_general(), cx))
            .child(field_row(
                s::settings_label_language(),
                crate::ui::select::select(&self.language_select, cx, 0),
            ))
            .child(field_row(
                s::settings_label_terminal_theme(),
                crate::ui::select::select(&self.terminal_preset_select, cx, 0),
            ))
            .child(field_row(s::settings_label_ui_theme(), ui_preset_widget))
            .child(field_row(
                s::settings_label_syntax_theme(),
                crate::ui::select::select(&self.syntax_theme_select, cx, 0),
            ))
            .child(checkbox_row(
                checkbox(
                    "settings-agent-use-reading-width",
                    s::settings_label_agent_use_reading_width(),
                    0,
                )
                .checked(self.agent_use_reading_width)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    if this.persist_bool_setting(BoolSetting::AgentUseReadingWidth, *checked, cx) {
                        this.agent_use_reading_width = *checked;
                        cx.notify();
                    }
                })),
            ))
            .into_any_element()
    }

    pub(super) fn render_font(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_font(), cx))
            .child(font_domain_label(s::settings_font_domain_terminal(), cx))
            .child(field_row(
                s::settings_label_font_family(),
                crate::ui::select::select(&self.terminal_font_family_select, cx, 0),
            ))
            .child(field_row(
                s::settings_label_font_size(),
                crate::ui::input(&self.terminal_font_size_input, cx, 0),
            ))
            .child(field_row(
                s::settings_label_line_height(),
                crate::ui::input(&self.terminal_line_height_input, cx, 0),
            ))
            .child(field_row(
                s::settings_label_cell_width(),
                crate::ui::input(&self.terminal_cell_width_input, cx, 0),
            ))
            .child(font_domain_label(s::settings_font_domain_editor(), cx))
            .child(field_row(
                s::settings_label_font_family(),
                crate::ui::select::select(&self.editor_font_family_select, cx, 0),
            ))
            .child(field_row(
                s::settings_label_font_size(),
                crate::ui::input(&self.editor_font_size_input, cx, 0),
            ))
            .child(field_row(
                s::settings_label_line_height(),
                crate::ui::input(&self.editor_line_height_input, cx, 0),
            ))
            .child(font_domain_label(s::settings_font_domain_agent_chat(), cx))
            .child(field_row(
                s::settings_label_font_family(),
                crate::ui::select::select(&self.agent_chat_font_family_select, cx, 0),
            ))
            .child(field_row(
                s::settings_label_font_size(),
                crate::ui::input(&self.agent_chat_font_size_input, cx, 0),
            ))
            .child(field_row(
                s::settings_label_line_height(),
                crate::ui::input(&self.agent_chat_line_height_input, cx, 0),
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
                crate::ui::select::select(&self.cursor_style_select, cx, 0),
            ))
            .child(checkbox_row(
                checkbox(
                    "settings-cursor-blinking",
                    s::settings_label_cursor_blinking(),
                    0,
                )
                .checked(cursor_blinking)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    if this.persist_bool_setting(BoolSetting::CursorBlinking, *checked, cx) {
                        this.cursor_blinking = *checked;
                        cx.notify();
                    }
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
                    0,
                )
                .checked(close_on_exit)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    if this.persist_bool_setting(BoolSetting::ShellClosePaneOnExit, *checked, cx) {
                        this.close_pane_on_exit = *checked;
                        cx.notify();
                    }
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
                crate::ui::input(&self.opacity_input, cx, 0),
            ))
            .child(checkbox_row(
                checkbox("settings-window-blur", s::settings_label_window_blur(), 0)
                    .checked(window_blur)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        if this.persist_bool_setting(BoolSetting::WindowBlur, *checked, cx) {
                            this.window_blur = *checked;
                            cx.notify();
                        }
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
                crate::ui::input(&self.scrollback_input, cx, 0),
            ))
            .child(field_row(
                s::settings_label_max_fps(),
                crate::ui::select::select(&self.max_fps_select, cx, 0),
            ))
            .child(field_row(
                s::settings_label_inset_x(),
                crate::ui::input(&self.inset_x_input, cx, 0),
            ))
            .child(field_row(
                s::settings_label_inset_y(),
                crate::ui::input(&self.inset_y_input, cx, 0),
            ))
            .into_any_element()
    }

    /// Left-dock (Sidebar/Files) and bottom-dock (macro grid) settings,
    /// combined into one page — both read as "dock configuration" to a
    /// user even though they're separate `Dock` instances internally.
    pub(super) fn render_dock(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let files_show_hidden = self.files_show_hidden;
        let files_use_gitignore = self.files_use_gitignore;
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_dock(), cx))
            .child(Self::section_label(s::settings_section_sidebar(), cx))
            .child(checkbox_row(
                checkbox("settings-show-hidden", s::settings_label_show_hidden(), 0)
                    .checked(files_show_hidden)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        if this.persist_bool_setting(BoolSetting::FilesShowHidden, *checked, cx) {
                            this.files_show_hidden = *checked;
                            cx.notify();
                        }
                    })),
            ))
            .child(checkbox_row(
                checkbox(
                    "settings-use-gitignore",
                    s::settings_label_use_gitignore(),
                    0,
                )
                .checked(files_use_gitignore)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    if this.persist_bool_setting(BoolSetting::FilesUseGitignore, *checked, cx) {
                        this.files_use_gitignore = *checked;
                        cx.notify();
                    }
                })),
            ))
            .child(Self::section_label(s::settings_section_panels(), cx))
            .child(field_row(
                s::settings_label_grid_columns(),
                crate::ui::input(&self.panels_grid_columns_input, cx, 0),
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
                crate::ui::input(&self.clipboard_streaming_input, cx, 0),
            ))
            .into_any_element()
    }

    pub(super) fn render_editor(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(
                s::settings_section_external_editor(),
                cx,
            ))
            .child(field_row(
                s::settings_label_preferred_editor(),
                crate::ui::select::select(&self.editor_select, cx, 0),
            ))
            .into_any_element()
    }

    /// Store the token typed into the field, then empty the field.
    fn save_telegram_token(&mut self, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) {
        self.save_telegram_token_with(crate::telegram::keychain::write_token, window, cx);
    }

    /// [`Self::save_telegram_token`] with the credential-store write supplied
    /// by the caller. A parameter because `keychain::write_token` reaches the
    /// real OS credential store with no test guard of its own (unlike
    /// `read_token`), so a test must be able to stand in for it.
    ///
    /// A failure lands in the log twice on purpose — once from the keychain
    /// layer with the raw tool stderr, once from here with the action the user
    /// was denied. Different questions when triaging.
    pub(super) fn save_telegram_token_with(
        &mut self,
        write: impl FnOnce(&str) -> std::io::Result<()>,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let token = self
            .telegram_token_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if token.is_empty() {
            return;
        }
        match write(&token) {
            Ok(()) => {
                self.telegram_token_configured = true;
                self.telegram_token_input.update(cx, |input, cx_state| {
                    input.set_value(String::new(), window, cx_state);
                });
                self.error = None;
                cx.notify();
            }
            // The token stays in the field: nothing was stored, so retrying is
            // the next thing the user will want and retyping it is not.
            Err(e) => self.report_section_error(
                s::settings_err_telegram_token_save(&e.to_string()),
                ErrorReport::new("Telegram token was not stored")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .dedup("settings.telegram.token_save"),
                cx,
            ),
        }
    }

    /// Forget the stored token, so the bridge has nothing to poll with.
    fn clear_telegram_token(&mut self, cx: &mut gpui::Context<Self>) {
        self.clear_telegram_token_with(crate::telegram::keychain::delete_token, cx);
    }

    /// [`Self::clear_telegram_token`] with the credential-store delete supplied
    /// by the caller — same reason as [`Self::save_telegram_token_with`].
    pub(super) fn clear_telegram_token_with(
        &mut self,
        delete: impl FnOnce() -> std::io::Result<()>,
        cx: &mut gpui::Context<Self>,
    ) {
        match delete() {
            Ok(()) => {
                self.telegram_token_configured = false;
                self.error = None;
                cx.notify();
            }
            Err(e) => self.report_section_error(
                s::settings_err_telegram_token_clear(&e.to_string()),
                ErrorReport::new("Telegram token was not removed")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .dedup("settings.telegram.token_clear"),
                cx,
            ),
        }
    }

    /// Drop the paired chat, so the bridge stops accepting that phone.
    ///
    /// Forced rather than conflict-checked: pairing is written by the bridge's
    /// poll loop, so `base_config` may legitimately be behind the live value
    /// and a conflict prompt here would only ask the user to confirm the
    /// pairing they are trying to remove.
    pub(super) fn unpair_telegram(&mut self, cx: &mut gpui::Context<Self>) {
        self.apply_settings_patch_force_as(
            daruda_config::SettingsPatch::TelegramAuthorizedChatId(None),
            s::settings_err_telegram_unpair,
            cx,
        );
    }

    pub(super) fn render_notifications(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let t = theme::current(cx);
        let body_color = t.text_primary;
        let telegram_enabled = self.telegram_enabled;
        let token_configured = self.telegram_token_configured;
        let authorized_chat_id = self.telegram_authorized_chat_id;
        let pair_code = self.telegram_pair_code.clone();
        // Read live rather than mirrored into a field: which daruda holds the
        // bot is not a setting, and the answer is only interesting while this
        // section is on screen.
        let held_elsewhere = crate::telegram::global::TelegramBridge::bot_held_elsewhere(cx);

        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_notifications(), cx))
            .child(self.remote_channel_settings.clone())
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(body_color)
                    .child(s::settings_telegram_heading()),
            )
            .when(held_elsewhere, |body| {
                body.child(crate::ui::alert::warning(
                    "settings-telegram-held-elsewhere",
                    s::settings_telegram_held_elsewhere(),
                ))
            })
            .child(checkbox_row(
                checkbox(
                    "settings-telegram-enabled",
                    s::settings_telegram_enabled_label(),
                    0,
                )
                .checked(telegram_enabled)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    if this.persist_bool_setting(BoolSetting::TelegramEnabled, *checked, cx) {
                        this.telegram_enabled = *checked;
                        cx.notify();
                    }
                })),
            ))
            .child(field_row(
                s::settings_telegram_token_label(),
                crate::ui::input(&self.telegram_token_input, cx, 0),
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
                                this.save_telegram_token(window, cx);
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
                                    this.clear_telegram_token(cx);
                                },
                            )),
                        )
                    }),
            )
            // Starts the pairing flow — comes right after the token is saved,
            // matching the real procedure (generate a code, then send it to
            // the bot via Telegram). Previously this button was rendered
            // *after* the paired-status/Check-Pairing row below, which read
            // backwards: a "Not paired" status with nothing yet to check.
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
                    ),
            )
            .when_some(pair_code, |body, code| {
                let copy_label = if self.telegram_pair_command_copy.copied() {
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
            // Pairing status — sits after the generate/send steps above, not
            // before them, plus Unpair (only meaningful once paired).
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
                    .when(authorized_chat_id.is_some(), |row| {
                        row.child(
                            button_danger(
                                "settings-telegram-unpair",
                                s::settings_telegram_unpair(),
                            )
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, _window, cx| {
                                    this.unpair_telegram(cx);
                                },
                            )),
                        )
                    }),
            )
            // The bot's own command menu. Registering it is a manual BotFather
            // step daruda cannot do for the user, so the block it needs is
            // here rather than only in the docs.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .child(
                        div()
                            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                            .text_color(body_color)
                            .child(s::settings_telegram_botfather_label()),
                    )
                    .child(
                        div()
                            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                            .text_color(body_color)
                            .child(s::settings_telegram_botfather_help()),
                    )
                    // A verbatim paste-me payload, so it gets the same
                    // monospace card a fenced code block does
                    // (`file_view_pane::render::markdown::block::code_surface`)
                    // rather than reading as prose.
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .bg(t.md_code_block_bg)
                            .border_1()
                            .border_color(t.border)
                            .rounded(px(theme::MD_CODE_BLOCK_RADIUS))
                            .px(px(theme::MD_CODE_BLOCK_PAD_X))
                            .py(px(theme::MD_CODE_BLOCK_PAD_Y))
                            .font(gpui::font("monospace"))
                            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                            .text_color(body_color)
                            .child(s::control_botfather_commands()),
                    )
                    .child(
                        div().flex().flex_row().child(
                            button(
                                "settings-telegram-copy-botfather",
                                if self.telegram_botfather_copy.copied() {
                                    s::error_modal_button_copied()
                                } else {
                                    s::error_modal_button_copy()
                                },
                            )
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, _window, cx| {
                                    this.copy_botfather_commands(cx);
                                },
                            )),
                        ),
                    ),
            )
            // Beside the bridge controls, not on a page of its own: `/daruda`
            // arrives over that bridge, so the two are one feature to the user.
            .child(self.render_orchestrator(cx))
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
                |this, _: &ClickEvent, _window, cx| {
                    this.open_config_file(cx);
                },
            )),
        )
    }

    /// Hand `config.toml` to the user's editor, creating its directory first.
    fn open_config_file(&mut self, cx: &mut gpui::Context<Self>) {
        self.open_config_file_with(|dir| std::fs::create_dir_all(dir), cx);
    }

    /// [`Self::open_config_file`] with the directory step supplied by the
    /// caller — the real one addresses `config_path()`, and the success branch
    /// hands a URL to the OS, neither of which a test may do.
    ///
    /// Returns whether the file was handed over. A failed directory step stops
    /// there: the path cannot name a file, so opening it would show the user an
    /// editor doing nothing instead of the reason.
    pub(in crate::settings) fn open_config_file_with(
        &mut self,
        ensure_dir: impl FnOnce(&std::path::Path) -> std::io::Result<()>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let path = daruda_config::config_path();
        if let Some(parent) = path.parent()
            && let Err(e) = ensure_dir(parent)
        {
            self.report_section_error(
                s::settings_err_open_config(&e.to_string()),
                ErrorReport::new(crate::surface::strings::error_create_config_dir_failed())
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("path", redact_home(parent))
                    .dedup("config.mkdir"),
                cx,
            );
            return false;
        }
        let url = match url::Url::from_file_path(&path) {
            Ok(url) => url,
            Err(()) => {
                let error = std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Config path cannot be represented as a file URL",
                );
                self.report_section_error(
                    s::settings_err_open_config(&error.to_string()),
                    ErrorReport::new("Config file URL could not be created")
                        .severity(ErrorSeverity::Warning)
                        .from_error(&error)
                        .at(file!(), line!())
                        .with_context("path", redact_home(&path))
                        .dedup("config.file_url"),
                    cx,
                );
                return false;
            }
        };
        cx.open_url(url.as_str());
        true
    }
}

#[cfg(test)]
#[allow(dead_code)] // exposed for tests that exercise the section without rendering it.
impl SettingsView {
    pub(in crate::settings) fn telegram_pair_command_copied(&self) -> bool {
        self.telegram_pair_command_copy.copied()
    }

    pub(in crate::settings) fn telegram_botfather_copied(&self) -> bool {
        self.telegram_botfather_copy.copied()
    }

    /// Test-only entry into [`Self::copy_botfather_commands`], for the same
    /// reason its pairing-code sibling below has one.
    pub(in crate::settings) fn copy_botfather_commands_for_test(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.copy_botfather_commands(cx);
    }

    /// Test-only entry into [`Self::copy_telegram_pair_command`] — the
    /// click handler that drives it lives inside a closure and isn't
    /// directly callable from tests. Mirrors
    /// `ErrorReportModal::copy_to_clipboard_for_test`.
    pub(in crate::settings) fn copy_telegram_pair_command_for_test(
        &mut self,
        code: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        self.copy_telegram_pair_command(code, cx);
    }
}
