//! What the hosting workspace may read off a chat pane. The fields stay
//! the view's own; a host that needs one more fact adds a reader here
//! rather than reaching in. Verbs live in `host_commands`.

use daruda_acp::ChatItem;
use daruda_store::project::PaneCwd;
use gpui::{Bounds, Pixels};

use super::super::session_config::SessionConfig;
use super::questions::{ComposerBinding, primary_fields};
use super::{AgentChatView, AgentSessionStatus, PromptId, PromptQueue, QueuedPrompt};

/// Longest question text a run report carries.
const QUESTION_SUMMARY_MAX_CHARS: usize = 120;

/// What the composer strip and placeholder say about the question the
/// composer answers. A summary only: the form itself stays the view's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionComposer {
    pub question_id: u64,
    /// Index of the focused field among the question's primary fields. A
    /// card pick can move it while the composer holds text: on a change the
    /// host commits that text to the field it last saw, by index.
    pub field: usize,
    /// `(field + 1, primary field count)`, for "1/2".
    pub position: (usize, usize),
    /// Open questions on this pane behind this one (E3), for "+1 waiting".
    pub waiting_after: usize,
    /// The focused field's title, or the question's message when it has
    /// none; the agent's words, verbatim.
    pub header: String,
    /// Typed text rides on the field's single pick as a note.
    pub note_mode: bool,
    /// The composer's Enter answers the question rather than prompting.
    pub bound: bool,
    /// The person bound the composer over a prompt draft, which now starts
    /// the answer; true until the next commit or bind.
    pub draft_became_answer: bool,
    /// The keyboard is on the card's send button. Composer text belongs to
    /// no field there, so the host commits it before and loads none.
    pub send_highlighted: bool,
    /// When the answer timeout declines the question, while it runs; the
    /// strip shows the time left through [`super::QuestionCountdown`].
    pub deadline: Option<std::time::Instant>,
}

/// `request` in one line: its first field's title, or its message when that
/// field has none, cut to [`QUESTION_SUMMARY_MAX_CHARS`].
pub fn question_summary(request: &daruda_acp::QuestionRequest) -> String {
    let text = primary_fields(request)
        .next()
        .and_then(|f| f.title.as_deref())
        .filter(|title| !title.is_empty())
        .unwrap_or(&request.message);
    let (prefix, cut) = daruda_core::text::char_prefix(text, QUESTION_SUMMARY_MAX_CHARS);
    if cut {
        format!("{prefix}…")
    } else {
        prefix.to_owned()
    }
}

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

    /// Whether the turn in flight is the phone's: a prompt relayed from it
    /// started the turn, and its outcome is owed back there.
    pub fn phone_owns_turn(&self) -> bool {
        self.phone_turn_state.is_some()
    }

    /// The oldest open question in one line for a run report: its first
    /// field's title, or the request message when that field has none.
    pub fn pending_question_summary(&self) -> Option<String> {
        Some(question_summary(&self.questions.first()?.request))
    }

    /// The oldest open question as the composer sees it, bound or not; only
    /// that question may take the composer.
    pub fn question_composer(&self) -> Option<QuestionComposer> {
        let question = self.questions.first()?;
        let field = question.focused_field();
        let header = field
            .and_then(|f| f.title.as_deref())
            .filter(|title| !title.is_empty())
            .unwrap_or(&question.request.message);
        Some(QuestionComposer {
            question_id: question.id,
            field: question.focus.field,
            position: (
                question.focus.field + 1,
                primary_fields(&question.request).count(),
            ),
            waiting_after: self.questions.open().count() - 1,
            header: header.to_owned(),
            note_mode: field.is_some_and(|f| question.is_note(f)),
            bound: question.composer.is_bound(),
            draft_became_answer: question.composer
                == ComposerBinding::Bound {
                    draft_became_answer: true,
                },
            send_highlighted: question.send_highlighted(),
            deadline: question.countdown(),
        })
    }

    /// The text committed to primary field `field` of the oldest open
    /// question, when that question is `id`. The composer loads it when the
    /// field takes the composer, so an earlier answer can be edited.
    pub fn question_field_text(&self, id: u64, field: usize) -> Option<String> {
        let question = self.questions.first().filter(|q| q.id == id)?;
        let target = primary_fields(&question.request).nth(field)?;
        let key = question.text_key(target)?;
        question.draft.text(key).map(str::to_owned)
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
