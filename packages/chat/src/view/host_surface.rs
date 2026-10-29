//! What the hosting workspace may read off a chat pane. The fields stay
//! the view's own; a host that needs one more fact adds a reader here
//! rather than reaching in. Verbs live in `host_commands`.

use daruda_acp::ChatItem;
use daruda_store::project::PaneCwd;
use gpui::{Bounds, Pixels};

use super::super::session_config::SessionConfig;
use super::{AgentChatView, AgentSessionStatus, PromptId, PromptQueue, QueuedPrompt};

impl AgentChatView {
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// The agent's display name, as the catalog last named it.
    pub fn agent_name(&self) -> &str {
        &self.agent_name
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn permission_session(&self) -> u64 {
        self.permission_session
    }

    pub fn session_title(&self) -> Option<&str> {
        self.session_title.as_deref()
    }

    /// The session's last-activity stamp, RFC 3339 as the agent sent it.
    pub fn session_updated_at(&self) -> Option<&str> {
        self.session_updated_at.as_deref()
    }

    pub fn status(&self) -> &AgentSessionStatus {
        &self.status
    }

    pub fn cwd(&self) -> Option<&PaneCwd> {
        self.cwd.as_ref()
    }

    /// Display detail only; activity and completion still use `activity_state`.
    /// Pending calls and subagent launch records do not prove tool execution.
    pub fn has_running_tool(&self) -> bool {
        self.is_busy() && running_tool_in_current_run(&self.items)
    }

    pub fn items(&self) -> &[ChatItem] {
        &self.items
    }

    pub fn session_config(&self) -> &SessionConfig {
        &self.session_config
    }

    pub fn queue(&self) -> &PromptQueue {
        &self.queue
    }

    /// Where the transcript list last painted, in window coordinates.
    pub fn list_bounds(&self) -> Option<Bounds<Pixels>> {
        self.list_bounds
    }

    pub fn dim_amount(&self) -> f32 {
        self.dim_amount
    }

    /// The model the user picked on this pane, which the next connect asks
    /// for over the agent's default.
    pub fn picked_model_id(&self) -> Option<&str> {
        self.picked_model_id.as_deref()
    }

    /// The adapter command the pane's option vocabularies are recorded
    /// under, once a connect has named it.
    pub fn agent_vocabulary_source(&self) -> Option<&str> {
        self.agent_vocabulary_source.as_deref()
    }

    /// The busy level `tick_activity` stored, without a fresh scan — so one
    /// pulse tick reads one consistent `now`.
    pub fn last_reconciled_busy(&self) -> bool {
        self.activity.span.is_busy()
    }
}

impl PromptQueue {
    /// The prompt ↑ edits: the live queue's newest, else the parked one's,
    /// since the strip renders parked rows as editable too.
    pub fn last_prompt_id(&self) -> Option<PromptId> {
        self.pending_prompts
            .last()
            .or_else(|| self.paused_prompts.last())
            .map(|q| q.id)
    }

    pub fn has_pending(&self) -> bool {
        !self.pending_prompts.is_empty()
    }

    /// A queued prompt by id, live or parked.
    pub fn find(&self, id: PromptId) -> Option<&QueuedPrompt> {
        self.in_strip_order().map(|(q, _)| q).find(|q| q.id == id)
    }

    /// The queued prompt the composer is editing, if any.
    pub fn editing(&self) -> Option<PromptId> {
        self.editing_prompt
    }

    /// Every queued prompt in the order the strip lists them, each with
    /// whether a Stop parked it. Parked prompts lead: they were submitted
    /// before anything queued after the Stop.
    pub fn in_strip_order(&self) -> impl Iterator<Item = (&QueuedPrompt, bool)> {
        let parked = self.paused_prompts.iter().map(|q| (q, true));
        parked.chain(self.pending_prompts.iter().map(|q| (q, false)))
    }
}

fn running_tool_in_current_run(items: &[ChatItem]) -> bool {
    items
        .iter()
        .rev()
        .take_while(|item| !matches!(item, ChatItem::UserText(_)))
        .any(|item| {
            matches!(item, ChatItem::ToolCall(tool)
            if tool.status == daruda_acp::ToolStatusView::InProgress && !tool.is_subagent_launch())
        })
}

#[cfg(test)]
mod tests;
