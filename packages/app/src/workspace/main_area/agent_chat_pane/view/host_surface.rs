//! What the hosting workspace may read off a chat pane. The fields stay
//! the view's own; a host that needs one more fact adds a reader here
//! rather than reaching in. Verbs live in `host_commands`.

use daruda_acp::ChatItem;
use daruda_store::project::PaneCwd;
use gpui::{Bounds, Pixels};

use super::super::session_config::SessionConfig;
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

    /// The model the user picked on this pane, which the next connect asks
    /// for over the agent's default.
    pub(in crate::workspace) fn picked_model_id(&self) -> Option<&str> {
        self.picked_model_id.as_deref()
    }

    /// The adapter command the pane's option vocabularies are recorded
    /// under, once a connect has named it.
    pub(in crate::workspace) fn agent_vocabulary_source(&self) -> Option<&str> {
        self.agent_vocabulary_source.as_deref()
    }

    /// The busy level `tick_activity` stored, without a fresh scan — so one
    /// pulse tick reads one consistent `now`.
    pub(in crate::workspace) fn last_reconciled_busy(&self) -> bool {
        self.activity.span.is_busy()
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

    pub(in crate::workspace) fn has_pending(&self) -> bool {
        !self.pending_prompts.is_empty()
    }

    /// A queued prompt by id, live or parked.
    pub(in crate::workspace) fn find(&self, id: PromptId) -> Option<&QueuedPrompt> {
        self.in_strip_order().map(|(q, _)| q).find(|q| q.id == id)
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
        assert_eq!(
            queue.find(queued(1, "").id).map(|q| q.text.as_str()),
            Some("parked")
        );
        assert!(queue.has_pending());
        queue.pending_prompts.clear();
        assert_eq!(queue.last_prompt_id(), Some(queued(1, "").id));
    }
}
