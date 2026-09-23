//! Grouped forms for the simple, specification-backed settings.

use gpui::{AnyElement, IntoElement, ParentElement as _};

use crate::settings::presentation::{card, page_stack};
use crate::settings::{BoolSetting as B, SelectSetting as S, SettingsView, TextSetting as T};
use crate::surface::strings as s;

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
            .child(card(s::settings_card_shell(), cx).child(self.switch_row(
                B::ShellClosePaneOnExit,
                s::settings_label_close_on_exit(),
                String::new(),
                cx,
            )))
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
            .child(card(s::settings_card_clipboard(), cx).child(self.text_row(
                T::ClipboardStreamingMaxBytes,
                s::settings_label_clipboard_streaming(),
                String::new(),
                cx,
            )))
            .into_any_element()
    }

    pub(in crate::settings) fn render_workspace(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        page_stack()
            .child(
                card(s::settings_section_sidebar(), cx)
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
                    )),
            )
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
