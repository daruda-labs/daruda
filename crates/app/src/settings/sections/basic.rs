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
            .child(self.select_row(S::Language, cx))
            .into_any_element()
    }

    pub(in crate::settings) fn render_appearance(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        page_stack()
            .child(
                card(s::settings_group_themes(), cx)
                    .child(self.select_row(S::UiPreset, cx))
                    .child(self.select_row(S::TerminalPreset, cx))
                    .child(self.select_row(S::SyntaxTheme, cx))
                    .child(crate::settings::presentation::config_only_row(
                        s::settings_label_custom_colors(),
                        s::settings_hint_custom_colors(),
                        "colors",
                        cx,
                    )),
            )
            .child(
                card(s::settings_group_window(), cx)
                    .child(self.text_row(T::WindowOpacity, cx))
                    .child(self.switch_row(B::WindowBlur, cx)),
            )
            .into_any_element()
    }

    pub(in crate::settings) fn render_font(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let mut body = page_stack();
        for (name, family, size, height) in font_domains() {
            let mut group = card(name(), cx)
                .child(self.select_row(family, cx))
                .child(self.text_row(size, cx))
                .child(self.text_row(height, cx));
            if family == S::TerminalFontFamily {
                group = group.child(self.text_row(T::TerminalCellWidth, cx));
            }
            body = body.child(group);
        }
        body.into_any_element()
    }

    pub(in crate::settings) fn render_terminal(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        page_stack()
            .child(
                card(s::settings_card_shell(), cx)
                    .child(self.text_row_wide(T::ShellProgram, cx))
                    .child(self.switch_row(B::ShellNaturalTextEditing, cx))
                    .child(self.switch_row(B::ShellClosePaneOnExit, cx))
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
                    .child(self.text_row(T::ScrollbackMaxRows, cx))
                    .child(self.select_row(S::RenderMaxFps, cx)),
            )
            .child(
                card(s::settings_group_insets(), cx)
                    .child(self.text_row(T::TerminalInsetX, cx))
                    .child(self.text_row(T::TerminalInsetY, cx)),
            )
            .child(card(s::settings_card_cursor(), cx).child(self.select_row(S::CursorStyle, cx)))
            .child(self.advanced_card(
                BuiltinSection::Terminal,
                vec![self.text_row(T::ClipboardStreamingMaxBytes, cx)],
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
                    .child(self.text_row(T::LeftDefaultWidth, cx))
                    .child(self.switch_row(B::LeftCollapsedByDefault, cx)),
            )
            .child(
                card(s::settings_card_files(), cx)
                    .child(self.switch_row(B::FilesShowHidden, cx))
                    .child(self.switch_row(B::FilesUseGitignore, cx))
                    .child(self.select_row(S::FileIconColorMode, cx))
                    .child(self.switch_row(B::PreviewTab, cx)),
            )
            .child(status_bar)
            .child(
                card(s::settings_section_panels(), cx)
                    .child(self.text_row(T::PanelsGridColumns, cx)),
            )
            .child(
                card(s::settings_card_external_editor(), cx)
                    .child(self.select_row(S::PreferredEditor, cx)),
            )
            .into_any_element()
    }
}

/// The status bar menu's label for `item`, reused so both places name it alike.
pub(in crate::settings) fn status_bar_item_label(item: daruda_config::StatusBarItem) -> String {
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
    pub(in crate::settings) fn status_bar_item_row(
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
