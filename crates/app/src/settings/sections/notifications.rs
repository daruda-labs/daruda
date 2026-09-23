//! The Notifications page: `[notifications]`, grouped by where the event
//! comes from so a terminal alert and an agent-chat alert never read alike.

use gpui::{AnyElement, IntoElement, ParentElement as _};

use crate::settings::presentation::{card, page_stack};
use crate::settings::{BoolSetting as B, SettingsView, TextSetting as T};
use crate::surface::strings as s;

impl SettingsView {
    pub(in crate::settings) fn render_notifications(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        page_stack()
            .child(
                card(s::settings_card_terminal_programs(), cx)
                    .child(self.switch_row(B::NotifyOsc9, cx))
                    .child(self.switch_row(B::NotifyOsc777, cx))
                    .child(self.switch_row(B::NotifyAttention, cx))
                    .child(self.switch_row(B::NotifyLongRunning, cx))
                    .child(self.dependent_rows(
                        B::NotifyLongRunning,
                        [self.text_row(T::NotifyLongRunningThresholdSecs, cx)],
                        cx,
                    )),
            )
            .child(
                card(s::settings_card_claude_code_terminals(), cx)
                    .child(self.switch_row(B::NotifyHook, cx)),
            )
            .child(
                card(s::settings_card_agent_chat_notifications(), cx)
                    .child(self.switch_row(B::NotifyAgentCompletion, cx))
                    .child(self.switch_row(B::NotifyAgentWaiting, cx)),
            )
            .child(
                card(s::settings_card_notify_behavior(), cx)
                    .child(self.switch_row(B::NotifySkipFocusedPane, cx)),
            )
            .into_any_element()
    }
}
