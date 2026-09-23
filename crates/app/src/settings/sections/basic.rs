//! Grouped forms for the simple, specification-backed settings.

use crate::settings::SettingsView;
use crate::surface::strings as s;

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
