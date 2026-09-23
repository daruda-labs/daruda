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
        let long_running = s::settings_label_notify_long_running();
        page_stack()
            .child(
                card(s::settings_card_terminal_programs(), cx)
                    .child(self.switch_row(
                        B::NotifyOsc9,
                        s::settings_label_notify_osc9(),
                        s::settings_hint_notify_osc9(),
                        cx,
                    ))
                    .child(self.switch_row(
                        B::NotifyOsc777,
                        s::settings_label_notify_osc777(),
                        s::settings_hint_notify_osc777(),
                        cx,
                    ))
                    .child(self.switch_row(
                        B::NotifyAttention,
                        s::settings_label_notify_attention(),
                        s::settings_hint_notify_attention(),
                        cx,
                    ))
                    .child(self.switch_row(
                        B::NotifyLongRunning,
                        long_running.clone(),
                        s::settings_hint_notify_long_running(),
                        cx,
                    ))
                    .child(self.dependent_rows(
                        B::NotifyLongRunning,
                        long_running,
                        [self.text_row(
                            T::NotifyLongRunningThresholdSecs,
                            s::settings_label_notify_long_running_threshold(),
                            String::new(),
                            cx,
                        )],
                        cx,
                    )),
            )
            .child(
                card(s::settings_card_claude_code_terminals(), cx).child(self.switch_row(
                    B::NotifyHook,
                    s::settings_label_notify_hook(),
                    s::settings_hint_notify_hook(),
                    cx,
                )),
            )
            .child(
                card(s::settings_card_agent_chat_notifications(), cx)
                    .child(self.switch_row(
                        B::NotifyAgentCompletion,
                        s::settings_label_notify_agent_completion(),
                        String::new(),
                        cx,
                    ))
                    .child(self.switch_row(
                        B::NotifyAgentWaiting,
                        s::settings_label_notify_agent_waiting(),
                        String::new(),
                        cx,
                    )),
            )
            .child(
                card(s::settings_card_notify_behavior(), cx).child(self.switch_row(
                    B::NotifySkipFocusedPane,
                    s::settings_label_notify_skip_focused(),
                    s::settings_hint_notify_skip_focused(),
                    cx,
                )),
            )
            .into_any_element()
    }
}
