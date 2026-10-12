//! What the hosting workspace may tell a chat pane. Each verb names the
//! change; the fields it touches stay the view's own.

use daruda_acp::{ChatItem, PermissionItem};
use daruda_store::project::PaneCwd;
use gpui::{Context, Task};

use super::super::transcript_defaults::TranscriptDefaults;
use super::AgentChatView;
use super::list_sync::ListSync;

impl AgentChatView {
    /// Rename the pane after the catalog renamed its agent.
    pub(in crate::workspace) fn set_agent_name(&mut self, name: String, cx: &mut Context<Self>) {
        if self.agent_name != name {
            self.agent_name = name;
            cx.notify();
        }
    }

    /// The phone recipient changed: forget which permission requests the
    /// old one was told about, so the next sweep tells the new one.
    pub(in crate::workspace) fn forget_permissions_told_to_phone(&mut self) {
        self.permissions_told_to_phone.clear();
    }

    /// The configured reading width changed. Only a pane laid out at that
    /// width has rows whose heights are now stale.
    pub(in crate::workspace) fn reading_width_changed(&mut self) {
        if self.content_width.is_reading() {
            self.apply_list_sync(ListSync::EveryRow, "reading_width_changed");
        }
    }

    /// Point the pane at another agent. The program belongs to the agent
    /// that reported it; keeping it would read the incoming agent's
    /// traffic in the outgoing one's dialect until the next connect.
    pub(in crate::workspace) fn switch_agent(
        &mut self,
        id: String,
        name: String,
        defaults: &TranscriptDefaults,
        cx: &mut Context<Self>,
    ) {
        self.agent_id = id;
        self.agent_name = name;
        self.agent_program = None;
        self.reseed_transcript_defaults(defaults, cx);
    }

    pub(in crate::workspace) fn set_agent_vocabulary_source(&mut self, source: String) {
        self.agent_vocabulary_source = Some(source);
    }

    pub(in crate::workspace) fn set_cwd(&mut self, cwd: PaneCwd) {
        self.cwd = Some(cwd);
    }

    /// Keep the session's event loop alive for as long as the pane is:
    /// closing the pane drops it, which ends the loop.
    pub(in crate::workspace) fn attach_event_pump(&mut self, pump: Task<()>) {
        self._event_pump = Some(pump);
    }

    /// The unresolved permission cards the phone has not been shown, in
    /// card order, after forgetting any it was shown that are since
    /// answered — so the record cannot outgrow the open requests.
    pub(in crate::workspace) fn take_permissions_untold_to_phone(&mut self) -> Vec<PermissionItem> {
        let pending = &self.pending_permissions;
        self.permissions_told_to_phone
            .retain(|id| pending.contains(id));
        self.permissions_untold_to_phone()
    }

    /// The same cards, without the clean-up — for the relay that runs
    /// as a request arrives.
    pub(in crate::workspace) fn permissions_untold_to_phone(&self) -> Vec<PermissionItem> {
        self.items
            .iter()
            .filter_map(|item| match item {
                ChatItem::Permission(p)
                    if p.resolved.is_none()
                        && self.pending_permissions.contains(&p.id)
                        && !self.permissions_told_to_phone.contains(&p.id) =>
                {
                    Some(p.clone())
                }
                _ => None,
            })
            .collect()
    }

    /// The phone has been shown this request; the periodic re-ask leaves
    /// it alone.
    pub(in crate::workspace) fn mark_permission_told_to_phone(&mut self, id: u64) {
        self.permissions_told_to_phone.insert(id);
    }

    /// Hold a permission request open, as an agent asking would — for a
    /// capture or a test that needs the awaiting state without a session.
    #[cfg(any(test, feature = "screenshot"))]
    pub(in crate::workspace) fn hold_permission_for_shot(&mut self, id: u64) {
        self.pending_permissions.insert(id);
    }

    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn set_session_id_for_shot(&mut self, id: &str) {
        self.session_id = Some(id.to_owned());
    }

    #[cfg(test)]
    pub(in crate::workspace) fn advertise_commands_for_test(
        &mut self,
        commands: Vec<daruda_acp::SlashCommand>,
    ) {
        self.session_config.available_commands = commands;
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::make_test_view;
    use daruda_acp::PermissionItem;

    #[gpui::test]
    fn a_rename_and_a_new_phone_reach_the_pane(cx: &mut gpui::TestAppContext) {
        let view = make_test_view(cx);
        view.update(cx, |v, _, cx| {
            v.hold_permission_for_shot(7);
            v.permissions_told_to_phone.insert(7);
            v.set_agent_name("Claude Code".into(), cx);
            v.forget_permissions_told_to_phone();
        })
        .unwrap();
        view.read_with(cx, |v, _| {
            assert_eq!(v.agent_name(), "Claude Code");
            assert!(v.permissions_told_to_phone.is_empty());
            assert!(
                v.pending_permissions.contains(&7),
                "only the phone's record is reset"
            );
        })
        .unwrap();
    }

    /// Shown once, a request is not re-asked; answered, it leaves the
    /// record too, so the record never outgrows the open requests.
    #[gpui::test]
    fn the_phone_is_asked_once_per_open_request(cx: &mut gpui::TestAppContext) {
        use super::super::tests::permission_card;
        let view = make_test_view(cx);
        view.update(cx, |v, _, _| {
            v.items = vec![permission_card(1), permission_card(2)];
            v.hold_permission_for_shot(1);
            v.hold_permission_for_shot(2);
            let ids = |cards: Vec<PermissionItem>| cards.iter().map(|p| p.id).collect::<Vec<_>>();
            assert_eq!(ids(v.permissions_untold_to_phone()), [1, 2]);
            v.mark_permission_told_to_phone(1);
            assert_eq!(ids(v.take_permissions_untold_to_phone()), [2]);
            v.pending_permissions.remove(&1);
            v.take_permissions_untold_to_phone();
            assert!(v.permissions_told_to_phone.is_empty());
        })
        .unwrap();
    }
}
