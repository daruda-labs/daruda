//! What the hosting workspace may read off a chat pane, and the few
//! things it may tell one. The fields stay the view's own; a host that
//! needs one more fact or verb adds it here rather than reaching in.

use daruda_acp::ChatItem;
use daruda_store::project::PaneCwd;
use gpui::{Bounds, Context, Pixels};

use super::super::session_config::SessionConfig;
use super::list_sync::ListSync;
use super::{AgentChatView, AgentSessionStatus, PromptId, PromptQueue, QueuedPrompt};

impl AgentChatView {
    pub(in crate::workspace) fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// The agent's display name, as the catalog last named it.
    pub(in crate::workspace) fn agent_name(&self) -> &str {
        &self.agent_name
    }

    pub(in crate::workspace) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(in crate::workspace) fn session_title(&self) -> Option<&str> {
        self.session_title.as_deref()
    }

    /// The session's last-activity stamp, RFC 3339 as the agent sent it.
    pub(in crate::workspace) fn session_updated_at(&self) -> Option<&str> {
        self.session_updated_at.as_deref()
    }

    pub(in crate::workspace) fn status(&self) -> &AgentSessionStatus {
        &self.status
    }

    pub(in crate::workspace) fn cwd(&self) -> Option<&PaneCwd> {
        self.cwd.as_ref()
    }

    pub(in crate::workspace) fn items(&self) -> &[ChatItem] {
        &self.items
    }

    pub(in crate::workspace) fn session_config(&self) -> &SessionConfig {
        &self.session_config
    }

    pub(in crate::workspace) fn queue(&self) -> &PromptQueue {
        &self.queue
    }

    /// Where the transcript list last painted, in window coordinates.
    pub(in crate::workspace) fn list_bounds(&self) -> Option<Bounds<Pixels>> {
        self.list_bounds
    }

    pub(in crate::workspace) fn dim_amount(&self) -> f32 {
        self.dim_amount
    }

    /// The busy level `tick_activity` stored, without a fresh scan — so one
    /// pulse tick reads one consistent `now`.
    pub(in crate::workspace) fn last_reconciled_busy(&self) -> bool {
        self.activity.span.is_busy()
    }
}

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

impl PromptQueue {
    /// The prompt ↑ edits: the live queue's newest, else the parked one's,
    /// since the strip renders parked rows as editable too.
    pub(in crate::workspace) fn last_prompt_id(&self) -> Option<PromptId> {
        self.pending_prompts
            .last()
            .or_else(|| self.paused_prompts.last())
            .map(|q| q.id)
    }

    /// The queued prompt the composer is editing, if any.
    pub(in crate::workspace) fn editing(&self) -> Option<PromptId> {
        self.editing_prompt
    }

    /// Every queued prompt in the order the strip lists them, each with
    /// whether a Stop parked it. Parked prompts lead: they were submitted
    /// before anything queued after the Stop.
    pub(in crate::workspace) fn in_strip_order(
        &self,
    ) -> impl Iterator<Item = (&QueuedPrompt, bool)> {
        let parked = self.paused_prompts.iter().map(|q| (q, true));
        parked.chain(self.pending_prompts.iter().map(|q| (q, false)))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::make_test_view;

    /// The readers answer from the constructor's arguments, which is the
    /// one state a host sees before any ACP event arrives.
    #[gpui::test]
    fn a_fresh_pane_reads_back_what_it_was_built_with(cx: &mut gpui::TestAppContext) {
        let view = make_test_view(cx);
        view.read_with(cx, |v, _| {
            assert_eq!(v.agent_id(), "claude");
            assert_eq!(v.agent_name(), "Claude");
            assert_eq!(v.session_id(), None);
            assert!(v.cwd().is_none());
            assert!(v.items().is_empty());
            assert!(!v.last_reconciled_busy());
        })
        .unwrap();
    }

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

    #[test]
    fn parked_prompts_lead_the_strip_and_the_live_queue_ends_it() {
        use super::super::tests::queued;
        let mut queue = super::PromptQueue {
            paused_prompts: vec![queued(1, "parked")],
            pending_prompts: vec![queued(2, "live")],
            ..super::PromptQueue::default()
        };
        let order: Vec<_> = queue
            .in_strip_order()
            .map(|(q, parked)| (q.text.as_str(), parked))
            .collect();
        assert_eq!(order, [("parked", true), ("live", false)]);
        assert_eq!(queue.last_prompt_id(), Some(queued(2, "").id));
        queue.pending_prompts.clear();
        assert_eq!(queue.last_prompt_id(), Some(queued(1, "").id));
    }
}
