//! Grouped forms for the simple, specification-backed settings.

use gpui::{AnyElement, IntoElement, ParentElement as _};

use crate::settings::presentation::{card, page_stack};
use crate::settings::{
    BoolSetting as B, SelectSetting as S, SettingsEvent, SettingsView, TextSetting as T,
};
use crate::surface::strings as s;
use daruda_config::BuiltinSection;

impl SettingsView {
    pub(in crate::settings) fn render_general(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        card(s::settings_group_language(), cx)
            .child(self.select_row(
                S::Language,
                s::settings_label_language(),
                s::settings_hint_user_scope(),
                cx,
            ))
            .into_any_element()
    }

    pub(in crate::settings) fn render_appearance(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        page_stack()
            .child(
                card(s::settings_group_themes(), cx)
                    .child(self.select_row(
                        S::UiPreset,
                        s::settings_label_ui_theme(),
                        s::settings_hint_ui_theme(),
                        cx,
                    ))
                    .child(self.select_row(
                        S::TerminalPreset,
                        s::settings_label_terminal_theme(),
                        s::settings_hint_terminal_theme(),
                        cx,
                    ))
                    .child(self.select_row(
                        S::SyntaxTheme,
                        s::settings_label_syntax_theme(),
                        s::settings_hint_syntax_theme(),
                        cx,
                    )),
            )
            .child(
                card(s::settings_group_window(), cx)
                    .child(self.text_row(
                        T::WindowOpacity,
                        s::settings_label_window_opacity(),
                        s::settings_hint_opacity(),
                        cx,
                    ))
                    .child(self.switch_row(
                        B::WindowBlur,
                        s::settings_label_window_blur(),
                        s::settings_hint_blur(),
                        cx,
                    )),
            )
            .into_any_element()
    }

    pub(in crate::settings) fn render_font(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let mut body = page_stack();
        for (name, family, size, height) in font_domains() {
            let mut group = card(name(), cx)
                .child(self.select_row(family, s::settings_label_font_family(), String::new(), cx))
                .child(self.text_row(size, s::settings_label_font_size(), String::new(), cx))
                .child(self.text_row(
                    height,
                    s::settings_label_line_height(),
                    s::settings_hint_line_height(),
                    cx,
                ));
            if family == S::TerminalFontFamily {
                group = group.child(self.text_row(
                    T::TerminalCellWidth,
                    s::settings_label_cell_width(),
                    s::settings_hint_cell_width(),
                    cx,
                ));
            }
            body = body.child(group);
        }
        body.into_any_element()
    }

    pub(in crate::settings) fn render_terminal(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        page_stack()
            .child(
                card(s::settings_card_shell(), cx)
                    .child(self.text_row_wide(
                        T::ShellProgram,
                        s::settings_label_shell_program(),
                        s::settings_hint_shell_program(),
                        cx,
                    ))
                    .child(self.switch_row(
                        B::ShellNaturalTextEditing,
                        s::settings_label_natural_text_editing(),
                        s::settings_hint_natural_text_editing(),
                        cx,
                    ))
                    .child(self.switch_row(
                        B::ShellClosePaneOnExit,
                        s::settings_label_close_on_exit(),
                        String::new(),
                        cx,
                    ))
                    .child(self.event_row(
                        "settings-open-project-config",
                        s::settings_label_project_shell(),
                        s::settings_hint_project_shell(),
                        s::settings_button_open_project_config(),
                        || SettingsEvent::OpenProjectConfig,
                        cx,
                    )),
            )
            .child(
                card(s::settings_group_rendering(), cx)
                    .child(self.text_row(
                        T::ScrollbackMaxRows,
                        s::settings_label_scrollback(),
                        s::settings_hint_scrollback(),
                        cx,
                    ))
                    .child(self.select_row(
                        S::RenderMaxFps,
                        s::settings_label_max_fps(),
                        String::new(),
                        cx,
                    )),
            )
            .child(
                card(s::settings_group_insets(), cx)
                    .child(self.text_row(
                        T::TerminalInsetX,
                        s::settings_label_inset_x(),
                        String::new(),
                        cx,
                    ))
                    .child(self.text_row(
                        T::TerminalInsetY,
                        s::settings_label_inset_y(),
                        String::new(),
                        cx,
                    )),
            )
            .child(card(s::settings_card_cursor(), cx).child(self.select_row(
                S::CursorStyle,
                s::settings_label_cursor_style(),
                s::settings_hint_cursor_style(),
                cx,
            )))
            .child(self.advanced_card(
                BuiltinSection::Terminal,
                vec![self.text_row(
                    T::ClipboardStreamingMaxBytes,
                    s::settings_label_clipboard_streaming(),
                    s::settings_hint_clipboard_streaming(),
                    cx,
                )],
                cx,
            ))
            .into_any_element()
    }

    pub(in crate::settings) fn render_workspace(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let mut status_bar = card(s::settings_card_status_bar(), cx);
        for item in daruda_config::StatusBarItem::ALL {
            status_bar = status_bar.child(self.status_bar_item_row(*item, cx));
        }
        page_stack()
            .child(
                card(s::settings_section_sidebar(), cx)
                    .child(self.text_row(
                        T::LeftDefaultWidth,
                        s::settings_label_left_default_width(),
                        s::settings_hint_new_windows_width(),
                        cx,
                    ))
                    .child(self.switch_row(
                        B::LeftCollapsedByDefault,
                        s::settings_label_left_collapsed(),
                        s::settings_hint_new_windows_state(),
                        cx,
                    )),
            )
            .child(
                card(s::settings_card_files(), cx)
                    .child(self.switch_row(
                        B::FilesShowHidden,
                        s::settings_label_show_hidden(),
                        String::new(),
                        cx,
                    ))
                    .child(self.switch_row(
                        B::FilesUseGitignore,
                        s::settings_label_use_gitignore(),
                        String::new(),
                        cx,
                    ))
                    .child(self.select_row(
                        S::FileIconColorMode,
                        s::settings_label_file_icon_colors(),
                        String::new(),
                        cx,
                    ))
                    .child(self.switch_row(
                        B::PreviewTab,
                        s::settings_label_preview_tab(),
                        s::settings_hint_preview_tab(),
                        cx,
                    )),
            )
            .child(status_bar)
            .child(card(s::settings_section_panels(), cx).child(self.text_row(
                T::PanelsGridColumns,
                s::settings_label_grid_columns(),
                String::new(),
                cx,
            )))
            .child(
                card(s::settings_card_external_editor(), cx).child(self.select_row(
                    S::PreferredEditor,
                    s::settings_label_preferred_editor(),
                    String::new(),
                    cx,
                )),
            )
            .into_any_element()
    }
}

/// The status bar menu's label for `item`, reused so both places name it alike.
fn status_bar_item_label(item: daruda_config::StatusBarItem) -> String {
    use daruda_config::StatusBarItem as I;
    match item {
        I::ProjectBranch => s::status_bar_toggle_project_branch(),
        I::AccountSlot => s::status_bar_toggle_account_slot(),
        I::Ports => s::status_bar_toggle_ports(),
        I::ClaudeUsage => s::status_bar_toggle_claude_usage(),
        I::Flow => s::status_bar_toggle_flow(),
    }
}

impl SettingsView {
    /// One status-bar segment's switch. It reads the list this window last
    /// saw and writes the whole list back, so a toggle from the bar's own
    /// menu in the meantime is adopted rather than flipped twice.
    fn status_bar_item_row(
        &self,
        item: daruda_config::StatusBarItem,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let shown = self.base_config.status_bar.is_visible(item);
        let label = status_bar_item_label(item);
        crate::settings::presentation::row(
            label.clone(),
            String::new(),
            crate::settings::presentation::switch_with_state(
                crate::ui::switch(
                    gpui::ElementId::Name(format!("settings-status-bar-{item:?}").into()),
                    shown,
                    cx,
                )
                .tooltip(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_status_bar_item_visible(item, !shown, cx)
                })),
                shown,
                cx,
            ),
            cx,
        )
    }

    pub(in crate::settings) fn set_status_bar_item_visible(
        &mut self,
        item: daruda_config::StatusBarItem,
        visible: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut hidden = self.base_config.status_bar.hidden_items.clone();
        hidden.retain(|i| *i != item);
        if !visible {
            hidden.push(item);
        }
        self.apply_settings_patch(
            daruda_config::SettingsPatch::StatusBarHiddenItems(hidden),
            cx,
        );
    }
}

type FontDomain = (fn() -> String, S, T, T);

fn font_domains() -> [FontDomain; 3] {
    [
        (
            s::settings_font_domain_terminal,
            S::TerminalFontFamily,
            T::TerminalFontSize,
            T::TerminalLineHeight,
        ),
        (
            s::settings_font_domain_editor,
            S::EditorFontFamily,
            T::EditorFontSize,
            T::EditorLineHeight,
        ),
        (
            s::settings_font_domain_agent_chat,
            S::AgentChatFontFamily,
            T::AgentChatFontSize,
            T::AgentChatLineHeight,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_domains_keep_their_own_size_and_spacing() {
        let domains = font_domains();
        assert_eq!(
            domains.map(|(_, f, _, _)| f),
            [
                S::TerminalFontFamily,
                S::EditorFontFamily,
                S::AgentChatFontFamily
            ]
        );
        assert_eq!(
            domains.map(|(_, _, size, _)| size),
            [T::TerminalFontSize, T::EditorFontSize, T::AgentChatFontSize]
        );
        assert_eq!(
            domains.map(|(_, _, _, line)| line),
            [
                T::TerminalLineHeight,
                T::EditorLineHeight,
                T::AgentChatLineHeight
            ]
        );
    }
}
